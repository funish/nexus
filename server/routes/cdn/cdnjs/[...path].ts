import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";

// Get appropriate cache-control header based on version
function getCdnjsCacheControl(versionSpecified: boolean): string {
  // When version not specified - shorter cache (10 minutes)
  if (!versionSpecified) {
    return "public, max-age=600";
  }
  // Specific versions - long cache (1 year, immutable)
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

  const versionSpecified = !!version;

  // If version not specified, fetch from cdnjs API
  if (!version) {
    const apiUrl = `https://api.cdnjs.com/libraries/${library}?fields=version,filename`;
    const apiRes = await fetch(apiUrl);

    if (!apiRes.ok) {
      throw new HTTPError({
        status: 404,
        statusText: "Library not found",
      });
    }

    const apiData = await apiRes.json();
    version = apiData.version;

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
        event.res.headers.set("Cache-Control", getCdnjsCacheControl(versionSpecified));

        return Buffer.from(defaultFileData);
      }
    }
  }

  // Get file from cache or GitHub
  const fileData = await getCachedFile(library, version, filepath);
  const contentType = getContentType(filepath || "application/octet-stream");

  event.res.headers.set("Content-Type", contentType);
  event.res.headers.set("Cache-Control", getCdnjsCacheControl(versionSpecified));

  return Buffer.from(fileData);
});
