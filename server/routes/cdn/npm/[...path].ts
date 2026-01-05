import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseTarGzip } from "nanotar";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";
import { calculateIntegrity } from "../../../utils/integrity";
import semver from "semver";
import type { CdnFile, CdnPackageListing } from "../../../utils/types";

defineRouteMeta({
  openAPI: {
    tags: ["CDN"],
    summary: "npm package CDN endpoint",
    description:
      "Access npm packages and their files. Supports package listings, metadata, and tarball downloads.",
    parameters: [
      {
        in: "path",
        name: "path",
        description: "Package path (e.g., 'package@version', 'package@version/file', 'package')",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description:
          "Successful response - returns package metadata, file listing, or tarball content",
      },
      404: {
        description: "Package not found",
      },
    },
  },
});

// Check if version is a complete semver (x.y.z) that should be cached long-term
function isCompleteSemver(version: string): boolean {
  // Complete semver: 1.2.3, 1.2.3-alpha.1, etc.
  return /^\d+\.\d+\.\d+/.test(version);
}

// Get appropriate cache-control header based on version
function getNpmCacheControl(version: string): string {
  // Incomplete semver or aliases - shorter cache (10 minutes)
  // Examples: "latest", "3", "3.1", "^1.2.3", "~1.2.3"
  if (!isCompleteSemver(version)) {
    return "public, max-age=600";
  }
  // Complete semver versions - long cache (1 year, immutable)
  return "public, max-age=31536000, immutable";
}

// Check if package metadata is already cached
async function isNpmPackageCached(packageName: string, version: string): Promise<boolean> {
  const storage = useStorage("cache");
  const cacheBase = `cdn/npm/${packageName}/${version}`;

  // Skip cache when version is incomplete semver
  if (!isCompleteSemver(version)) {
    return false;
  }

  // For complete semver versions, check if already cached
  const cachedMeta = await storage.getMeta(cacheBase);
  return !!cachedMeta?.files;
}

// Get a single file from cache or fetch from tarball
async function getOrCacheNpmFile(
  packageName: string,
  version: string,
  tarballUrl: string,
  filepath: string,
): Promise<Uint8Array> {
  const storage = useStorage("cache");
  const cacheKey = `cdn/npm/${packageName}/${version}/${filepath}`;

  // Try cache first
  const cached = await storage.getItemRaw(cacheKey);
  if (cached) {
    return new Uint8Array(cached);
  }

  // Cache miss - download and extract tarball to get the file
  const tarballRes = await fetch(tarballUrl);
  if (!tarballRes.ok) {
    throw new HTTPError({
      status: 502,
      statusText: "Failed to download tarball",
    });
  }

  const tarballData = await tarballRes.bytes();
  const files = await parseTarGzip(tarballData);

  // Determine root directory in tarball
  const firstFile = files[0];
  const rootDir = firstFile?.name.split("/")[0] || "package";
  const rootPath = `${rootDir}/`;

  // Find and return the requested file
  const targetFile = files.find((f) => {
    if (f.type !== "file" || !f.data) return false;
    const relativePath = f.name.slice(rootPath.length);
    return relativePath === filepath;
  });

  if (!targetFile?.data) {
    throw new HTTPError({
      status: 404,
      statusText: `File not found: ${filepath}`,
    });
  }

  return targetFile.data;
}

// Background task to cache all files from an npm package version
async function cacheNpmPackageInBackground(
  packageName: string,
  version: string,
  tarballUrl: string,
) {
  try {
    const storage = useStorage("cache");
    const cacheBase = `cdn/npm/${packageName}/${version}`;

    // Check if already cached
    const cachedMeta = await storage.getMeta(cacheBase);
    if (cachedMeta?.files) {
      return; // Already cached
    }

    // Download and extract tarball
    const tarballRes = await fetch(tarballUrl);
    if (!tarballRes.ok) {
      console.error(`Failed to download npm tarball for ${packageName}@${version}`);
      return;
    }

    const tarballData = await tarballRes.bytes();
    const files = await parseTarGzip(tarballData);

    // Determine root directory in tarball
    const firstFile = files[0];
    const rootDir = firstFile?.name.split("/")[0] || "package";
    const rootPath = `${rootDir}/`;

    // Filter files to cache
    const filesToCache = files.filter((f) => f.type === "file" && f.data);

    // Build file list metadata (synchronous)
    const fileList: Array<CdnFile> = filesToCache.map((f) => ({
      name: f.name.slice(rootPath.length),
      size: f.size || 0,
    }));

    // Optimization: Concurrent caching using Promise.allSettled
    const cachePromises = filesToCache.map(async (file) => {
      const relativePath = file.name.slice(rootPath.length);
      const cacheKey = `${cacheBase}/${relativePath}`;

      try {
        // Check if already cached
        const exists = await storage.getItemRaw(cacheKey);
        if (exists) {
          return; // Skip if already exists
        }

        // Cache file data
        if (file.data) {
          await storage.setItemRaw(cacheKey, file.data);

          // Calculate SHA-256 integrity for SRI
          const integrity = await calculateIntegrity(file.data);

          // Update file list with integrity
          const fileItem = fileList.find((f) => f.name === relativePath);
          if (fileItem) {
            fileItem.integrity = integrity;
          }
        }
      } catch (error) {
        console.error(`Failed to cache file ${relativePath}:`, error);
        // Continue with other files even if this one fails
      }
    });

    // Wait for all cache operations to complete (even if some fail)
    await Promise.allSettled(cachePromises);

    // Store file list in meta
    await storage.setMeta(cacheBase, { files: fileList });
  } catch (error) {
    console.error(`Background cache failed for ${packageName}@${version}:`, error);
  }
}

/**
 * CDN npm route handler
 *
 * Supported path formats:
 * - Scoped:   @scope/package@version/path or @scope/package/path
 * - Normal:    package@version/path or package/path
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");
  // Check original URL to detect trailing slash
  const originalUrl = event.req.url || "";
  const hasTrailingSlash = originalUrl.endsWith("/");

  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  // Parse path: @scope/package or package
  let packageName: string;
  let version: string;
  let filepath: string;

  if (path.startsWith("@")) {
    // Scoped: @types/hast@latest/index.d.ts
    const match = path.match(/^@([^/]+)\/([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
    if (!match) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid scoped package path format",
      });
    }

    const [, scope, pkg, pkgVersion, pkgFilepath] = match;
    packageName = `@${scope}/${pkg}`;
    version = pkgVersion || "latest";
    filepath = pkgFilepath || "";
  } else {
    // Normal: uikit@latest/dist/js/uikit.js
    const match = path.match(/^([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
    if (!match) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid package path format",
      });
    }

    const [, pkg, pkgVersion, pkgFilepath] = match;
    if (!pkg) {
      throw new HTTPError({
        status: 400,
        statusText: "Package name is required",
      });
    }
    packageName = pkg;
    version = pkgVersion || "latest";
    filepath = pkgFilepath || "";
  }

  // Fetch package metadata from npm registry
  const registryUrl = "https://registry.npmjs.org";
  const metadataRes = await fetch(`${registryUrl}/${packageName}`);
  if (!metadataRes.ok) {
    throw new HTTPError({
      status: 404,
      statusText: "Package not found",
    });
  }

  const metadata = await metadataRes.json();
  const distTags = metadata["dist-tags"];

  // Try to get specified version, fallback to latest if not found
  let versionInfo = metadata.versions[version];

  // If exact version not found, try to resolve version range (e.g., "3", "3.1", "^3.1.0")
  if (!versionInfo) {
    const allVersions = Object.keys(metadata.versions);

    // Try semver range matching
    const matchedVersion = semver.maxSatisfying(allVersions, version);
    if (matchedVersion) {
      version = matchedVersion;
      versionInfo = metadata.versions[version];
    }
  }

  // Fallback to latest if still not found
  if (!versionInfo && distTags?.latest) {
    const latest = distTags.latest;
    if (latest) {
      versionInfo = metadata.versions[latest];
      version = latest;
    }
  }

  if (!versionInfo) {
    throw new HTTPError({
      status: 404,
      statusText: "Version not found",
    });
  }

  // Download tarball info
  const tarballUrl = versionInfo.dist.tarball;

  const storage = useStorage("cache");
  const cacheBase = `cdn/npm/${packageName}/${version}`;

  // Check if entire package is already cached
  const isCached = await isNpmPackageCached(packageName, version);

  // Handle +esm bundling request
  if (filepath === "+esm") {
    // Determine entry point from package.json
    let entryFile = versionInfo.browser || versionInfo.main || versionInfo.module || "index.js";

    // Ensure package is fully cached
    if (!isCached) {
      await cacheNpmPackageInBackground(packageName, version, tarballUrl);
    }

    // Bundle the package
    const { bundleNpmPackage } = await import("../../../utils/bundler");
    const bundledCode = await bundleNpmPackage({
      packageName,
      version,
      entryPoint: entryFile,
    });

    event.res.headers.set("Content-Type", "application/javascript; charset=utf-8");
    event.res.headers.set("Cache-Control", "public, max-age=31536000, immutable");
    event.res.headers.set("X-ESM-Version", version);

    return bundledCode;
  }

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/npm/uikit/ -> list directory contents with metadata
      // If not cached, wait for entire package to be cached
      if (!isCached) {
        await cacheNpmPackageInBackground(packageName, version, tarballUrl);
      }

      const meta = await storage.getMeta(cacheBase);
      const fileList = (meta?.files || []) as Array<CdnFile>;

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      const response: CdnPackageListing = {
        // Package metadata
        name: metadata.name,
        version: version,
        // Directory info
        path: "",
        files: fileList,
      };

      return response;
    } else {
      // /cdn/npm/uikit -> return entry file
      // Use npm registry metadata to determine entry file
      // Priority: browser > main > module > index.js
      let entryFile = versionInfo.browser || versionInfo.main || versionInfo.module || "index.js";

      const fileData = await getOrCacheNpmFile(packageName, version, tarballUrl, entryFile);

      // Trigger background caching for entire package
      if (!isCached) {
        event.waitUntil(cacheNpmPackageInBackground(packageName, version, tarballUrl));
      }

      const contentType = getContentType(entryFile);

      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getNpmCacheControl(version));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache or tarball
  try {
    const fileData = await getOrCacheNpmFile(packageName, version, tarballUrl, filepath);

    // Trigger background caching for entire package
    if (!isCached) {
      event.waitUntil(cacheNpmPackageInBackground(packageName, version, tarballUrl));
    }

    // Return file content
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getNpmCacheControl(version));

    return Buffer.from(fileData);
  } catch (error) {
    // If file not found, try to list as directory
    if ((error as HTTPError).statusCode === 404) {
      // Package must be cached to list directories
      if (!isCached) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}. Package not yet cached.`,
        });
      }

      const meta = await storage.getMeta(cacheBase);
      const allFiles =
        (meta?.files as Array<{ name: string; type: string; size: number }> | undefined) || [];

      // Filter files by directory prefix
      const dirPrefix = `${filepath}/`;
      const dirContents: CdnFile[] = allFiles
        .filter((file: { name: string }) => file.name.startsWith(dirPrefix))
        .map((file: { name: string; size: number }) => ({
          name: file.name.slice(dirPrefix.length),
          size: file.size,
        }))
        .filter((file: { name: string }) => file.name.length > 0);

      if (dirContents.length === 0) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}`,
        });
      }

      dirContents.sort((a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name));

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      const response: CdnPackageListing = {
        name: metadata.name,
        version: version,
        path: filepath,
        files: dirContents,
      };

      return response;
    }

    throw error;
  }
});
