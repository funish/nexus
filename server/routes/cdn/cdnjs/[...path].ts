import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";
import semver from "semver";
import type { CdnFile, CdnPackageListing } from "../../../utils/types";

defineRouteMeta({
  openAPI: {
    tags: ["CDN"],
    summary: "cdnjs library CDN",
    description: "Access libraries hosted on cdnjs CDN",
    parameters: [
      {
        in: "path",
        name: "path",
        description: "Library path (e.g., 'jquery@3.6.0/dist/jquery.min.js')",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Returns library file with appropriate content-type",
      },
      404: {
        description: "Library not found",
      },
    },
  },
});

// Check if version is a complete semver (x.y.z) that should be cached long-term
function isCompleteSemver(version: string): boolean {
  // Complete semver: 1.2.3, v1.2.3, 1.2.3-alpha.1, etc.
  // Remove 'v' prefix if present, then check for x.y.z format
  const normalizedVersion = version.replace(/^v/, "");
  return /^\d+\.\d+\.\d+/.test(normalizedVersion);
}

// Get appropriate cache-control header based on version
function getCdnjsCacheControl(version: string): string {
  // Incomplete semver or aliases - shorter cache (10 minutes)
  // Examples: "latest", "3", "3.7", "v3.7"
  if (!isCompleteSemver(version)) {
    return "public, max-age=600";
  }
  // Complete semver versions - long cache (1 year, immutable)
  // Examples: "3.7.1", "v3.7.1", "3.7.1-beta"
  return "public, max-age=31536000, immutable";
}

// Get file from cache or fetch from GitHub
async function getCachedFile(
  library: string,
  version: string,
  filepath: string,
): Promise<Uint8Array> {
  const storage = useStorage("cache");
  const cacheKey = `cdn/cdnjs/${library}/${version}/${filepath}`;

  // Try to get from cache
  const cached = await storage.getItemRaw(cacheKey);
  if (cached) {
    return new Uint8Array(cached);
  }

  // Cache miss - fetch from GitHub
  const rawUrl = `https://raw.githubusercontent.com/cdnjs/cdnjs/refs/heads/master/ajax/libs/${library}/${version}/${filepath}`;
  const fileRes = await fetch(rawUrl);

  if (!fileRes.ok) {
    throw new HTTPError({
      status: 404,
      statusText: "File not found",
    });
  }

  const fileData = await fileRes.bytes();

  // Store in cache for future requests
  await storage.setItemRaw(cacheKey, fileData);

  return fileData;
}

// Get file list for a library version from cdnjs API and cache in meta
async function ensureCdnjsFileListCached(library: string, version: string): Promise<string[]> {
  const storage = useStorage("cache");
  const cacheBase = `cdn/cdnjs/${library}/${version}`;

  // Check if already cached
  const cachedMeta = await storage.getMeta(cacheBase);
  if (cachedMeta?.files) {
    return cachedMeta.files as string[];
  }

  // Fetch from cdnjs API
  const apiUrl = `https://api.cdnjs.com/libraries/${library}/${version}`;
  const apiRes = await fetch(apiUrl);

  if (!apiRes.ok) {
    throw new HTTPError({
      status: 404,
      statusText: "Library version not found",
    });
  }

  const apiData = await apiRes.json();
  const fileList = (apiData.files as string[]) || [];

  // Store in meta for future use
  await storage.setMeta(cacheBase, { files: fileList });

  return fileList;
}

// Get file list from cache
async function getCachedFileList(library: string, version: string): Promise<string[]> {
  const storage = useStorage("cache");
  const cacheBase = `cdn/cdnjs/${library}/${version}`;

  const meta = await storage.getMeta(cacheBase);
  return (meta?.files as string[]) || [];
}

/**
 * CDN cdnjs route handler
 *
 * Supported path formats:
 * - With @ separator: library@version/path
 * - Original cdnjs format: library/version/path
 *
 * Examples:
 * - /cdn/cdnjs/jquery@3.6.0/jquery.min.js
 * - /cdn/cdnjs/jquery/3.6.0/jquery.min.js
 * - /cdn/cdnjs/jquery (uses API to get latest version and default file)
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");
  // Check original URL to detect trailing slash
  const originalUrl = event.req.url || "";
  const hasTrailingSlash = originalUrl.endsWith("/");

  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  let library: string;
  let version: string;
  let filepath: string;

  // Try @ format first: library@version/file
  const atMatch = path.match(/^([^@/]+)@([^/]+)(?:\/(.*))?$/);
  if (atMatch) {
    const [, lib, libVersion, libFilepath] = atMatch;
    if (!lib) {
      throw new HTTPError({
        status: 400,
        statusText: "Library name is required",
      });
    }
    library = lib;
    version = libVersion ?? "";
    filepath = libFilepath ?? "";

    // If version is "latest", treat as unspecified (will fetch from API)
    if (version === "latest") {
      version = "";
    }
  } else {
    // Original cdnjs format: library/version/file or library/file
    const parts = path.split("/");

    if (parts.length === 0 || !parts[0]) {
      throw new HTTPError({
        status: 400,
        statusText: "Library name is required",
      });
    }

    library = parts[0];

    // Check if second part looks like a version number
    // Version patterns: 1.2.3, v1.2.3, 1.2, etc.
    if (parts.length >= 2) {
      const second = parts[1];
      if (!second) {
        throw new HTTPError({
          status: 400,
          statusText: "Invalid path format",
        });
      }

      // If second part is "latest", treat as unspecified version
      if (second === "latest") {
        version = "";
        filepath = parts.slice(2).join("/");
      } else {
        const versionPattern = /^v?\d+\.\d+(\.\d+)?(-[^/]+)?$/;

        if (versionPattern.test(second)) {
          // Second part is a version: library/version/file
          version = second;
          filepath = parts.slice(2).join("/");
        } else {
          // Second part is not a version: library/file (use latest)
          version = "";
          filepath = parts.slice(1).join("/");
        }
      }
    } else {
      // Only library name provided
      version = "";
      filepath = "";
    }
  }

  // If version not specified or incomplete, fetch from cdnjs API to get complete version
  if (!version || !isCompleteSemver(version)) {
    const apiUrl = `https://api.cdnjs.com/libraries/${library}?fields=version,versions,filename`;
    const apiRes = await fetch(apiUrl);

    if (!apiRes.ok) {
      throw new HTTPError({
        status: 404,
        statusText: "Library not found",
      });
    }

    const apiData = await apiRes.json();

    // Extract version strings from API
    // cdnjs API returns: { versions: ["1.2.3", "1.2.4", ...], version: "1.2.4" }
    const allVersions = apiData.versions as string[] | undefined;
    if (allVersions && allVersions.length > 0) {
      // Try to find version in the list (exact match)
      let versionFound = allVersions.includes(version);

      // If exact version not found, try to resolve version range (e.g., "3", "3.7", "^3.7.0")
      if (!versionFound && version) {
        const matchedVersion = semver.maxSatisfying(allVersions, version);
        if (matchedVersion) {
          version = matchedVersion;
          versionFound = true;
        }
      }

      // If still not found or version not specified, get latest version
      if (!versionFound) {
        // Sort by semver (descending) and get first
        const sortedVersions = allVersions.sort(semver.rcompare);
        const latestVersion = sortedVersions[0];
        if (latestVersion) {
          version = latestVersion;
        }
      }
    } else {
      // Fallback to API's default version if versions list not available
      version = apiData.version;
    }

    // If filepath not specified, use default filename from API
    if (!filepath) {
      filepath = apiData.filename || "";
    }
  }

  // If filepath is empty and no trailing slash, it's a library root access
  // Fetch default file from API
  if (!filepath && !hasTrailingSlash) {
    const apiUrl = `https://api.cdnjs.com/libraries/${library}?fields=filename`;
    const apiRes = await fetch(apiUrl);

    if (apiRes.ok) {
      const apiData = await apiRes.json();
      const defaultFile = apiData.filename;

      if (defaultFile) {
        const defaultFileData = await getCachedFile(library, version, defaultFile);
        const contentType = getContentType(defaultFile);

        event.res.headers.set("Content-Type", contentType);
        event.res.headers.set("Cache-Control", getCdnjsCacheControl(version));

        return Buffer.from(defaultFileData);
      }
    }
  }

  // If filepath is empty with trailing slash, list all files
  if (!filepath && hasTrailingSlash) {
    const fileList = await ensureCdnjsFileListCached(library, version);

    const files: CdnFile[] = fileList.map((file) => ({
      name: file,
      size: 0, // cdnjs API doesn't provide file sizes
    }));

    event.res.headers.set("Content-Type", "application/json");
    event.res.headers.set("Cache-Control", "public, max-age=600");

    const response: CdnPackageListing = {
      name: library,
      version: version,
      path: "",
      files,
    };

    return response;
  }

  // Try to get file from cache or GitHub
  try {
    const fileData = await getCachedFile(library, version, filepath);
    const contentType = getContentType(filepath);

    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getCdnjsCacheControl(version));

    return Buffer.from(fileData);
  } catch (error) {
    // If file not found, check if it's a directory path
    if ((error as HTTPError).statusCode === 404) {
      const fileList = await getCachedFileList(library, version);

      // Filter files by directory prefix
      const dirPrefix = `${filepath}/`;
      const dirContents: CdnFile[] = fileList
        .filter((file) => file.startsWith(dirPrefix))
        .map((file) => ({
          name: file.slice(dirPrefix.length),
          size: 0,
        }))
        .filter((file) => file.name.length > 0);

      if (dirContents.length === 0) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}`,
        });
      }

      dirContents.sort((a, b) => a.name.localeCompare(b.name));

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      const response: CdnPackageListing = {
        name: library,
        version: version,
        path: filepath,
        files: dirContents,
      };

      return response;
    }

    throw error;
  }
});
