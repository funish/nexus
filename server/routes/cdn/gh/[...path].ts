import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseTarGzip } from "nanotar";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";
import semver from "semver";
import type { CdnFile, CdnPackageListing } from "../../../utils/types";

// Check if version is a branch (not a tag or commit)
function isBranch(version: string): boolean {
  // Explicit branch names
  if (["main", "master", "dev"].includes(version)) {
    return true;
  }
  // Full commit hash (40 characters)
  if (/^[a-f0-9]{40}$/.test(version)) {
    return false;
  }
  // Default to treating as tag (not branch)
  return false;
}

// Check if version is a complete semver tag (x.y.z)
function isCompleteSemver(version: string): boolean {
  // Ensure version is a string
  if (typeof version !== "string") {
    return false;
  }
  // Complete semver: 1.2.3, v1.2.3, 1.2.3-beta.1, etc.
  // Remove 'v' prefix if present, then check for x.y.z format
  const normalizedVersion = version.replace(/^v/, "");
  return /^\d+\.\d+\.\d+/.test(normalizedVersion);
}

// Get appropriate cache-control header based on version type
function getCacheControl(version: string): string {
  // Branches, incomplete versions - shorter cache (10 minutes)
  if (isBranch(version) || !isCompleteSemver(version)) {
    return "public, max-age=600";
  }
  // Complete semver tags and commits - long cache (1 year, immutable)
  return "public, max-age=31536000, immutable";
}

// Construct GitHub tarball URL using codeload
async function getGitHubTarballUrl(owner: string, repo: string, version: string): Promise<string> {
  // Full commit hash (40 characters)
  if (/^[a-f0-9]{40}$/.test(version)) {
    return `https://codeload.github.com/${owner}/${repo}/tar.gz/${version}`;
  }

  // Explicit branch names
  if (["main", "master", "dev"].includes(version)) {
    return `https://codeload.github.com/${owner}/${repo}/tar.gz/refs/heads/${version}`;
  }

  // Default as tag
  return `https://codeload.github.com/${owner}/${repo}/tar.gz/refs/tags/${version}`;
}

// Ensure all files from a GitHub repository version are cached
async function ensureGitHubCached(owner: string, repo: string, version: string) {
  const storage = useStorage("cache");
  const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;

  // Skip cache when version is incomplete semver or branch - always refetch
  if (isBranch(version) || !isCompleteSemver(version)) {
    // Delete old cache if exists
    await storage.removeItem(cacheBase);
  } else {
    // For complete semver tags and commits, check if already cached
    const cachedMeta = await storage.getMeta(cacheBase);
    if (cachedMeta?.files) {
      return;
    }
  }

  // Get tarball URL
  const tarballUrl = await getGitHubTarballUrl(owner, repo, version);

  // Download and extract tarball
  const tarballRes = await fetch(tarballUrl);
  if (!tarballRes.ok) {
    throw new HTTPError({
      status: 502,
      statusText: "Failed to download GitHub tarball",
    });
  }

  const tarballData = await tarballRes.bytes();
  const files = await parseTarGzip(tarballData);

  // Determine root directory in tarball
  // GitHub format: {repo}-{ref}/ e.g., basis-main/
  // Skip pax_global_header which is metadata, not actual content
  const firstContentFile = files.find(
    (f) => f.name.includes("/") && !f.name.startsWith("pax_global_header"),
  );
  const rootDir = firstContentFile?.name.split("/")[0] || `${repo}-`;
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
  owner: string,
  repo: string,
  version: string,
  filepath: string,
): Promise<Uint8Array | null> {
  const storage = useStorage("cache");
  const cacheKey = `cdn/gh/${owner}/${repo}/${version}/${filepath}`;
  const cached = await storage.getItemRaw(cacheKey);

  if (!cached) {
    return null;
  }

  return new Uint8Array(cached);
}

/**
 * CDN GitHub route handler
 *
 * Supported path formats:
 * - user/repo@version/path or user/repo/path
 *
 * Examples:
 * - /cdn/gh/vuejs/core@v3.4.0/packages/vue/src/index.ts
 * - /cdn/gh/npm/cli@latest/dist/index.js
 * - /cdn/gh/tailwindlabs/tailwindcss@v3.4.0/src/index.ts
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");
  // Check original URL to detect trailing slash
  const originalUrl = event.req.url || "";
  const hasTrailingSlash = originalUrl.endsWith("/");

  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  // Parse path: user/repo@version/path or user/repo/path
  const match = path.match(/^([^/]+)\/([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
  if (!match) {
    throw new HTTPError({
      status: 400,
      statusText: "Invalid GitHub repository path format",
    });
  }

  const [, owner, repo, repoVersion, filepath] = match;

  if (!owner || !repo) {
    throw new HTTPError({
      status: 400,
      statusText: "Owner and repository are required",
    });
  }

  let version = repoVersion || "main";

  // If version not specified or incomplete, fetch from jsDelivr API to get complete version
  if (!repoVersion || !isCompleteSemver(version)) {
    const apiUrl = `https://data.jsdelivr.com/v1/packages/gh/${owner}/${repo}`;
    const apiRes = await fetch(apiUrl);

    if (apiRes.ok) {
      const apiData = await apiRes.json();

      // Extract version strings from objects
      // jsDelivr API returns: { versions: [{ version: "5.3.8" }, { version: "5.3.7" }, ...] }
      const versionObjects = apiData.versions as Array<{ version: string }> | undefined;
      const allVersions =
        versionObjects?.map((v) => v.version).filter((v): v is string => typeof v === "string") ||
        [];

      // Try to find version in the list (exact match)
      let versionFound = allVersions.includes(version);

      // If exact version not found, try to resolve version range (e.g., "5", "5.3", "^5.3.0")
      if (!versionFound && repoVersion) {
        const matchedVersion = semver.maxSatisfying(allVersions, repoVersion);
        if (matchedVersion) {
          version = matchedVersion;
          versionFound = true;
        }
      }

      // If still not found or version not specified, get latest version
      if (!versionFound && allVersions.length > 0) {
        // Sort by semver (descending) and get first
        const sortedVersions = allVersions.sort(semver.rcompare);
        const latestVersion = sortedVersions[0];
        if (latestVersion) {
          version = latestVersion;
        }
      }
    }
  }

  // Download tarball and cache all files
  await ensureGitHubCached(owner, repo, version);

  const storage = useStorage("cache");
  const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;

  // Special handling for repository root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/gh/vuejs/core/ -> list directory contents
      const meta = await storage.getMeta(cacheBase);
      const fileList = (meta?.files || []) as Array<CdnFile>;

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      const response: CdnPackageListing = {
        name: `${owner}/${repo}`,
        version: version,
        path: "",
        files: fileList,
      };

      return response;
    } else {
      // /cdn/gh/vuejs/core -> return README or main file
      // GitHub doesn't have package.json, so we try README.md or index.js
      const readmeData = await getCachedFile(owner, repo, version, "README.md");

      if (readmeData) {
        const contentType = getContentType("README.md");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        return Buffer.from(readmeData);
      }

      // Fallback to index.js or list directory
      const indexData = await getCachedFile(owner, repo, version, "index.js");

      if (indexData) {
        const contentType = getContentType("index.js");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        return Buffer.from(indexData);
      }

      // If no README or index, return 404
      throw new HTTPError({
        status: 404,
        statusText: "No entry file found (README.md or index.js)",
      });
    }
  }

  // For non-root paths, try to get file from cache
  const fileData = await getCachedFile(owner, repo, version, filepath);

  // If file found, return content
  if (fileData) {
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getCacheControl(version));

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
    name: `${owner}/${repo}`,
    version: version,
    path: filepath,
    files: dirContents,
  };

  return response;
});
