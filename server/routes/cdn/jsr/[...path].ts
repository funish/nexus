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
    summary: "JSR (JavaScript Registry) CDN",
    description: "Access packages from the JSR registry for JavaScript/TypeScript modules",
    parameters: [
      {
        in: "path",
        name: "path",
        description: "JSR package path (e.g., '@std/package@version/file')",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Returns package file or listing",
      },
      404: {
        description: "Package not found",
      },
    },
  },
});

// Check if version is a complete semver (x.y.z) that should be cached long-term
function isCompleteSemver(version: string): boolean {
  // Complete semver: 1.0.0, 1.0.0-alpha.1, etc.
  return /^\d+\.\d+\.\d+/.test(version);
}

// Get appropriate cache-control header based on version
function getJsrCacheControl(version: string): string {
  // Incomplete semver or aliases - shorter cache (10 minutes)
  // Examples: "latest", "1", "1.1", "^1.1.0", "~1.1.0"
  if (!isCompleteSemver(version)) {
    return "public, max-age=600";
  }
  // Complete semver versions - long cache (1 year, immutable)
  return "public, max-age=31536000, immutable";
}

// Ensure all files from a package version are cached
async function ensurePackageCached(packageName: string, version: string, tarballUrl: string) {
  const storage = useStorage("cache");
  const cacheBase = `cdn/jsr/${packageName}/${version}`;

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
    if (file.data) {
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
  const cacheKey = `cdn/jsr/${packageName}/${version}/${filepath}`;
  const cached = await storage.getItemRaw(cacheKey);

  if (!cached) {
    return null;
  }

  return new Uint8Array(cached);
}

/**
 * CDN JSR route handler
 *
 * Supported path formats:
 * - Scoped:   @scope/package@version/path or @scope/package/path
 *
 * Examples:
 * - /cdn/jsr/@luca/flag@1.0.0/main.ts
 * - /cdn/jsr/@std/path@latest/mod.ts
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");
  // Check original URL to detect trailing slash
  const originalUrl = event.req.url || "";
  const hasTrailingSlash = originalUrl.endsWith("/");

  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  // Parse path: @scope/package or @scope/package@version
  let packageName: string;
  let version: string;
  let filepath: string;

  // JSR packages always have scope
  const match = path.match(/^@([^/]+)\/([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
  if (!match) {
    throw new HTTPError({
      status: 400,
      statusText: "Invalid JSR package path format",
    });
  }

  const [, scope, pkg, pkgVersion, pkgFilepath] = match;
  packageName = `@${scope}/${pkg}`;
  version = pkgVersion || "latest";
  filepath = pkgFilepath || "";

  // Fetch package metadata from npm.jsr.io
  // JSR uses npm compatibility name: @scope/package -> @jsr/scope__package
  const npmCompatName = `@jsr/${scope}__${pkg}`;
  const registryUrl = "https://npm.jsr.io";
  const metadataRes = await fetch(`${registryUrl}/${npmCompatName}`);
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

  // If exact version not found, try to resolve version range (e.g., "1", "1.1", "^1.1.0")
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
  const cacheBase = `cdn/jsr/${packageName}/${version}`;

  // Read package.json to get exports field for entry file
  let entryFile = "mod.ts"; // Default fallback
  const packageJsonData = await getCachedFile(packageName, version, "package.json");
  if (packageJsonData) {
    try {
      const packageJson = JSON.parse(new TextDecoder().decode(packageJsonData));
      const exports = packageJson.exports;

      if (typeof exports === "string") {
        // "./mod.js" -> "mod.js"
        entryFile = exports.startsWith("./") ? exports.slice(2) : exports;
      } else if (exports && exports["."]) {
        // { ".": { "default": "./mod.js" } } or { ".": "./mod.js" }
        const dotExport = exports["."];
        if (typeof dotExport === "string") {
          entryFile = dotExport.startsWith("./") ? dotExport.slice(2) : dotExport;
        } else if (dotExport && dotExport.default) {
          entryFile = dotExport.default.startsWith("./")
            ? dotExport.default.slice(2)
            : dotExport.default;
        }
      }
    } catch {
      // If package.json is invalid, use default mod.ts
    }
  }

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/jsr/@luca/flag/ -> list directory contents with metadata
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
      // /cdn/jsr/@luca/flag -> return entry file
      const fileData = await getCachedFile(packageName, version, entryFile);

      if (!fileData) {
        throw new HTTPError({
          status: 404,
          statusText: `Entry file not found: ${entryFile}`,
        });
      }

      const contentType = getContentType(entryFile);

      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getJsrCacheControl(version));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache
  const fileData = await getCachedFile(packageName, version, filepath);

  // If file found, return content
  if (fileData) {
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getJsrCacheControl(version));

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
