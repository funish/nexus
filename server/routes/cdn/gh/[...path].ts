import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam, HTTPError } from "nitro/h3";
import semver from "semver";

import {
  type CdnFile,
  type CdnPackageListing,
  getCacheControl,
  getContentType,
  getDirectoryListing,
  isPackageCached,
  cachePackageFromTarball,
  extractFileFromTarball,
} from "../../../utils/cdn";
import { cacheStorage } from "../../../utils/storage";

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

// Construct GitHub tarball URL using codeload
function getGitHubTarballUrl(owner: string, repo: string, version: string): string {
  // Full commit hash (40 characters)
  if (/^[a-f0-9]{40}$/.test(version)) {
    return `https://codeload.github.com/${owner}/${repo}/tar.gz/${version}`;
  }

  // Non-semver versions are treated as branch refs
  if (!semver.valid(version)) {
    return `https://codeload.github.com/${owner}/${repo}/tar.gz/refs/heads/${version}`;
  }

  // Default as tag
  return `https://codeload.github.com/${owner}/${repo}/tar.gz/refs/tags/${version}`;
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

  let version = repoVersion || "";

  // If version not specified, try jsDelivr API to get latest tag.
  // If version is specified but incomplete, try to resolve via API.
  // Skip API for valid semver — already exact.
  if (!repoVersion || !semver.valid(version)) {
    const apiUrl = `https://data.jsdelivr.com/v1/packages/gh/${owner}/${repo}`;
    const apiRes = await fetch(apiUrl);

    if (apiRes.ok) {
      const apiData = await apiRes.json();

      // Extract version strings from objects
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

      // Only fall back to latest tag when user didn't specify a version
      if (!versionFound && allVersions.length > 0 && !repoVersion) {
        const sortedVersions = allVersions.filter((v) => semver.valid(v)).sort(semver.rcompare);
        const latestVersion = sortedVersions[0];
        if (latestVersion) {
          version = latestVersion;
        }
      }
    }
  }

  // Fallback to main branch if jsDelivr API has no version data
  if (!version) {
    version = "main";
  }

  // Normalize semver versions with v-prefix for GitHub (tags always use v-prefix)
  if (semver.valid(version) && !version.startsWith("v")) {
    version = `v${version}`;
  }

  const tarballUrl = getGitHubTarballUrl(owner, repo, version);
  const storage = cacheStorage;
  const cacheBase = `cdn/gh/${owner}/${repo}/${version}`;
  const rootDirOptions = { skipPaxHeaders: true, fallbackName: `${repo}-` };

  // Construct raw URL for direct single-file access
  const rawBaseUrl = `https://raw.githubusercontent.com/${owner}/${repo}/${version}`;
  const getRawUrl = (fp: string) => `${rawBaseUrl}/${fp}`;

  // Check if entire package is already cached
  const cacheable = semver.valid(version) !== null;
  const isCached = await isPackageCached(cacheBase, cacheable);

  // Special handling for repository root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/gh/vuejs/core/ -> list directory contents
      // If not cached, wait for entire package to be cached
      if (!isCached) {
        await cachePackageFromTarball(
          tarballUrl,
          cacheBase,
          rootDirOptions,
          `gh:${owner}/${repo}@${version}`,
        );
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
      let fileData = await extractFileFromTarball(
        tarballUrl,
        "README.md",
        `${cacheBase}/README.md`,
        rootDirOptions,
        getRawUrl("README.md"),
      );

      if (fileData) {
        const contentType = getContentType("README.md");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        // Trigger background caching for entire package
        if (!isCached) {
          event.waitUntil(
            cachePackageFromTarball(
              tarballUrl,
              cacheBase,
              rootDirOptions,
              `gh:${owner}/${repo}@${version}`,
            ),
          );
        }

        return Buffer.from(fileData);
      }

      // Fallback to index.js
      try {
        fileData = await extractFileFromTarball(
          tarballUrl,
          "index.js",
          `${cacheBase}/index.js`,
          rootDirOptions,
          getRawUrl("index.js"),
        );

        const contentType = getContentType("index.js");

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCacheControl(version));

        // Trigger background caching for entire package
        if (!isCached) {
          event.waitUntil(
            cachePackageFromTarball(
              tarballUrl,
              cacheBase,
              rootDirOptions,
              `gh:${owner}/${repo}@${version}`,
            ),
          );
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
    const fileData = await extractFileFromTarball(
      tarballUrl,
      filepath,
      `${cacheBase}/${filepath}`,
      rootDirOptions,
      getRawUrl(filepath),
    );

    // Trigger background caching for entire package
    if (!isCached) {
      event.waitUntil(
        cachePackageFromTarball(
          tarballUrl,
          cacheBase,
          rootDirOptions,
          `gh:${owner}/${repo}@${version}`,
        ),
      );
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

      const listing = await getDirectoryListing(cacheBase, filepath, `${owner}/${repo}`, version);
      if (!listing) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}`,
        });
      }

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      return listing;
    }

    throw error;
  }
});
