import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";
import { calculateIntegrity } from "../../../utils/integrity";
import semver from "semver";
import type { CdnFile, CdnPackageListing } from "../../../utils/types";

defineRouteMeta({
  openAPI: {
    tags: ["CDN"],
    summary: "GitHub releases CDN",
    description: "Access GitHub release assets and repository files",
    parameters: [
      {
        in: "path",
        name: "path",
        description: "GitHub resource path (owner/repo/version/file or owner/repo/file)",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Returns file content or package listing",
      },
      404: {
        description: "Resource not found",
      },
    },
  },
});

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

// Check if package metadata is already cached
async function isGitHubCached(owner: string, repo: string, version: string): Promise<boolean> {
  const storage = useStorage("cache");
  const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;

  // Skip cache when version is incomplete semver or branch
  if (isBranch(version) || !isCompleteSemver(version)) {
    return false;
  }

  // For complete semver tags and commits, check if already cached
  const cachedMeta = await storage.getMeta(cacheBase);
  return !!cachedMeta?.files;
}

// Get a single file from cache or fetch from tarball
async function getOrCacheFile(
  owner: string,
  repo: string,
  version: string,
  filepath: string,
): Promise<Uint8Array> {
  const storage = useStorage("cache");
  const cacheKey = `cdn/gh/${owner}/${repo}/${version}/${filepath}`;

  // Try cache first
  const cached = await storage.getItemRaw(cacheKey);
  if (cached) {
    return new Uint8Array(cached);
  }

  // Cache miss - download and extract tarball to get the file
  const tarballUrl = await getGitHubTarballUrl(owner, repo, version);
  const tarballRes = await fetch(tarballUrl);
  if (!tarballRes.ok) {
    throw new HTTPError({
      status: 502,
      statusText: "Failed to download GitHub tarball",
    });
  }

  const tarballData = await tarballRes.bytes();
  const archive = new Bun.Archive(tarballData);
  const filesMap = await archive.files();

  // Determine root directory in tarball
  const firstContentPath = [...filesMap.keys()].find(
    (path) => path.includes("/") && !path.startsWith("pax_global_header"),
  );
  const rootDir = firstContentPath?.split("/")[0] || `${repo}-`;
  const rootPath = `${rootDir}/`;

  // Find and return the requested file
  let targetFileData: Uint8Array | undefined;
  for (const [path, file] of filesMap) {
    const relativePath = path.slice(rootPath.length);
    if (relativePath === filepath) {
      targetFileData = await file.bytes();
      break;
    }
  }

  if (!targetFileData) {
    throw new HTTPError({
      status: 404,
      statusText: `File not found: ${filepath}`,
    });
  }

  return targetFileData;
}

// Background task to cache all files from a GitHub repository version
async function cacheGitHubPackageInBackground(owner: string, repo: string, version: string) {
  try {
    const storage = useStorage("cache");
    const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;

    // Check if already cached
    const cachedMeta = await storage.getMeta(cacheBase);
    if (cachedMeta?.files) {
      return; // Already cached
    }

    // Get tarball URL
    const tarballUrl = await getGitHubTarballUrl(owner, repo, version);

    // Download and extract tarball
    const tarballRes = await fetch(tarballUrl);
    if (!tarballRes.ok) {
      console.error(`Failed to download GitHub tarball for ${owner}/${repo}@${version}`);
      return;
    }

    const tarballData = await tarballRes.bytes();
    const archive = new Bun.Archive(tarballData);
    const filesMap = await archive.files();

    // Determine root directory in tarball
    const firstContentPath = [...filesMap.keys()].find(
      (path) => path.includes("/") && !path.startsWith("pax_global_header"),
    );
    const rootDir = firstContentPath?.split("/")[0] || `${repo}-`;
    const rootPath = `${rootDir}/`;

    // Build file list metadata
    const fileList: Array<CdnFile> = [];
    for (const [path, file] of filesMap) {
      fileList.push({
        name: path.slice(rootPath.length),
        size: file.size,
      });
    }

    // Optimization: Concurrent caching using Promise.allSettled
    const cachePromises = Array.from(filesMap.entries()).map(async ([path, file]) => {
      const relativePath = path.slice(rootPath.length);
      const cacheKey = `${cacheBase}/${relativePath}`;

      try {
        // Check if already cached
        const exists = await storage.getItemRaw(cacheKey);
        if (exists) {
          return; // Skip if already exists
        }

        // Cache file data
        const fileData = await file.bytes();
        await storage.setItemRaw(cacheKey, fileData);

        // Calculate SHA-256 integrity for SRI
        const integrity = await calculateIntegrity(fileData);

        // Update file list with integrity
        const fileItem = fileList.find((f) => f.name === relativePath);
        if (fileItem) {
          fileItem.integrity = integrity;
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
    console.error(`Background cache failed for ${owner}/${repo}@${version}:`, error);
  }
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

  const storage = useStorage("cache");
  const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;

  // Check if entire package is already cached
  const isCached = await isGitHubCached(owner, repo, version);

  // Special handling for repository root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/gh/vuejs/core/ -> list directory contents
      // If not cached, wait for entire package to be cached
      if (!isCached) {
        await cacheGitHubPackageInBackground(owner, repo, version);
      }

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
      // Get file (from cache or tarball)
      let fileData = await getOrCacheFile(owner, repo, version, "README.md");

      if (fileData) {
        const contentType = getContentType("README.md");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        // Trigger background caching for entire package
        if (!isCached) {
          event.waitUntil(cacheGitHubPackageInBackground(owner, repo, version));
        }

        return Buffer.from(fileData);
      }

      // Fallback to index.js
      try {
        fileData = await getOrCacheFile(owner, repo, version, "index.js");

        const contentType = getContentType("index.js");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        // Trigger background caching for entire package
        if (!isCached) {
          event.waitUntil(cacheGitHubPackageInBackground(owner, repo, version));
        }

        return Buffer.from(fileData);
      } catch {
        // If no README or index, return 404
        throw new HTTPError({
          status: 404,
          statusText: "No entry file found (README.md or index.js)",
        });
      }
    }
  }

  // For non-root paths, try to get file from cache or tarball
  try {
    const fileData = await getOrCacheFile(owner, repo, version, filepath);

    // Trigger background caching for entire package
    if (!isCached) {
      event.waitUntil(cacheGitHubPackageInBackground(owner, repo, version));
    }

    // Return file content
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getCacheControl(version));

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
        name: `${owner}/${repo}`,
        version: version,
        path: filepath,
        files: dirContents,
      };

      return response;
    }

    throw error;
  }
});
