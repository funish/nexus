import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseTarGzip } from "nanotar";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";

// Check if version should not be cached (when version not specified)
function isNonCacheableVersion(versionSpecified: boolean): boolean {
  return !versionSpecified;
}

// Get appropriate cache-control header based on version
function getJsrCacheControl(versionSpecified: boolean): string {
  // When version not specified - shorter cache (10 minutes)
  if (isNonCacheableVersion(versionSpecified)) {
    return "public, max-age=600";
  }
  // Specific versions - long cache (1 year, immutable)
  return "public, max-age=31536000, immutable";
}

// Ensure all files from a package version are cached
async function ensurePackageCached(
  packageName: string,
  version: string,
  tarballUrl: string,
  versionSpecified: boolean,
) {
  const storage = useStorage("cache");
  const cacheBase = `cdn/jsr/${packageName}/${version}`;

  // Skip cache when version not specified - always refetch
  if (isNonCacheableVersion(versionSpecified)) {
    await storage.removeItem(cacheBase);
  } else {
    // For specific versions, check if already cached
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
  const fileList: Array<{ name: string; type: string; size: number }> = [];

  // Cache all files
  for (const file of files) {
    if (file.data) {
      // Remove root directory from path
      const relativePath = file.name.slice(rootPath.length);
      const cacheKey = `${cacheBase}/${relativePath}`;
      await storage.setItemRaw(cacheKey, file.data);

      fileList.push({
        name: relativePath,
        type: "file",
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
  let versionSpecified: boolean;

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
  versionSpecified = !!pkgVersion;
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
  await ensurePackageCached(packageName, version, tarballUrl, versionSpecified);

  const storage = useStorage("cache");
  const cacheBase = `cdn/jsr/${packageName}/${version}`;

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/jsr/@luca/flag/ -> list directory contents
      const meta = await storage.getMeta(cacheBase);
      const fileList = meta?.files || [];

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      return {
        path: "",
        type: "directory",
        files: fileList,
      };
    } else {
      // /cdn/jsr/@luca/flag -> return package.json main file
      const packageJsonData = await getCachedFile(packageName, version, "package.json");

      if (!packageJsonData) {
        throw new HTTPError({
          status: 404,
          statusText: "package.json not found",
        });
      }

      const packageJson = JSON.parse(new TextDecoder().decode(packageJsonData));

      // Determine entry file following npm priority
      let entryFile =
        packageJson.jsdelivr ||
        packageJson.browser ||
        packageJson.main ||
        packageJson.module ||
        "index.js";

      // Handle exports field
      if (
        !packageJson.jsdelivr &&
        !packageJson.browser &&
        !packageJson.main &&
        packageJson.exports
      ) {
        if (typeof packageJson.exports === "string") {
          entryFile = packageJson.exports;
        } else if (packageJson.exports["."]) {
          const exportEntry = packageJson.exports["."];
          entryFile =
            typeof exportEntry === "string" ? exportEntry : exportEntry.default || entryFile;
        }
      }

      const fileData = await getCachedFile(packageName, version, entryFile);

      if (!fileData) {
        throw new HTTPError({
          status: 404,
          statusText: `Entry file not found: ${entryFile}`,
        });
      }

      const contentType = getContentType(entryFile);

      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getJsrCacheControl(versionSpecified));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache
  const fileData = await getCachedFile(packageName, version, filepath);

  // If file found, return content
  if (fileData) {
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getJsrCacheControl(versionSpecified));

    return Buffer.from(fileData);
  }

  // If file not found, try to list as directory
  const meta = await storage.getMeta(cacheBase);
  const allFiles =
    (meta?.files as Array<{ name: string; type: string; size: number }> | undefined) || [];

  // Filter files by directory prefix
  const dirPrefix = `${filepath}/`;
  const dirContents = allFiles
    .filter((file: { name: string }) => file.name.startsWith(dirPrefix))
    .map((file: { name: string; type: string; size: number }) => ({
      name: file.name.slice(dirPrefix.length),
      type: file.type,
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

  return {
    path: filepath,
    type: "directory",
    files: dirContents,
  };
});
