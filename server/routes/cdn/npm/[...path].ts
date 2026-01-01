import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseTarGzip } from "nanotar";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";
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

// Ensure all files from a package version are cached
async function ensurePackageCached(packageName: string, version: string, tarballUrl: string) {
  const storage = useStorage("cache");
  const cacheBase = `cdn/npm/${packageName}/${version}`;

  // Skip cache when version is incomplete semver - always refetch
  if (!isCompleteSemver(version)) {
    await storage.removeItem(cacheBase);
  } else {
    // For complete semver versions, check if already cached
    const cachedMeta = await storage.getMeta(cacheBase);
    if (cachedMeta?.files) {
      return;
    }
  }

  // Download and extract tarball
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

  // Build file list
  const fileList: Array<CdnFile> = [];

  // Cache all files
  for (const file of files) {
    if (file.type === "file" && file.data) {
      // Remove root directory from path
      const relativePath = file.name.slice(rootPath.length);
      const cacheKey = `${cacheBase}/${relativePath}`;
      await storage.setItemRaw(cacheKey, file.data);

      fileList.push({
        name: relativePath,
        size: file.size || 0,
      });
    }
  }

  // Store file list in meta
  await storage.setMeta(cacheBase, { files: fileList });
}

// Get a single file from cache
async function getCachedFile(
  packageName: string,
  version: string,
  filepath: string,
): Promise<Uint8Array | null> {
  const storage = useStorage("cache");
  const cacheKey = `cdn/npm/${packageName}/${version}/${filepath}`;
  const cached = await storage.getItemRaw(cacheKey);

  if (!cached) {
    return null;
  }

  return new Uint8Array(cached);
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

  // Download tarball and cache all files
  const tarballUrl = versionInfo.dist.tarball;
  await ensurePackageCached(packageName, version, tarballUrl);

  const storage = useStorage("cache");
  const cacheBase = `cdn/npm/${packageName}/${version}`;

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/npm/uikit/ -> list directory contents with metadata
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

      const fileData = await getCachedFile(packageName, version, entryFile);

      if (!fileData) {
        throw new HTTPError({
          status: 404,
          statusText: `Entry file not found: ${entryFile}`,
        });
      }

      const contentType = getContentType(entryFile);

      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getNpmCacheControl(version));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache
  const fileData = await getCachedFile(packageName, version, filepath);

  // If file found, return content
  if (fileData) {
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getNpmCacheControl(version));

    return Buffer.from(fileData);
  }

  // If file not found, try to list as directory
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
});
