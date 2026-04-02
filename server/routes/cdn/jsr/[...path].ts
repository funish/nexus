import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam, HTTPError } from "nitro/h3";
import semver from "semver";

import {
  type CdnFile,
  type CdnPackageListing,
  JSR_REGISTRY_URL,
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
  const metadataRes = await fetch(`${JSR_REGISTRY_URL}/${npmCompatName}`);
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

  // Download tarball info
  const tarballUrl = versionInfo.dist.tarball;

  const storage = cacheStorage;
  const cacheBase = `cdn/jsr/${packageName}/${version}`;

  // Check if entire package is already cached
  const cacheable = semver.valid(version) !== null;
  const isCached = await isPackageCached(cacheBase, cacheable);

  // Read package.json to get exports field for entry file
  let entryFile = "mod.ts"; // Default fallback
  const packageJsonData = await extractFileFromTarball(
    tarballUrl,
    "package.json",
    `${cacheBase}/package.json`,
  );
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

  // Trigger background caching for entire package (if not already cached)
  if (!isCached) {
    event.waitUntil(
      cachePackageFromTarball(tarballUrl, cacheBase, undefined, `jsr:${packageName}@${version}`),
    );
  }

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/jsr/@luca/flag/ -> list directory contents with metadata
      // If not cached, wait for entire package to be cached
      if (!isCached) {
        await cachePackageFromTarball(
          tarballUrl,
          cacheBase,
          undefined,
          `jsr:${packageName}@${version}`,
        );
      }

      const meta = await storage.getMeta(cacheBase);
      const fileList = (meta?.files || []) as Array<CdnFile>;

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      const response: CdnPackageListing = {
        name: metadata.name,
        version: version,
        path: "",
        files: fileList,
      };

      return response;
    } else {
      // /cdn/jsr/@luca/flag -> return entry file
      const fileData = await extractFileFromTarball(
        tarballUrl,
        entryFile,
        `${cacheBase}/${entryFile}`,
      );

      if (!fileData) {
        throw new HTTPError({
          status: 404,
          statusText: `Entry file not found: ${entryFile}`,
        });
      }

      const contentType = getContentType(entryFile);
      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getCacheControl(version));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache or tarball
  try {
    const fileData = await extractFileFromTarball(tarballUrl, filepath, `${cacheBase}/${filepath}`);

    const contentType = getContentType(filepath);
    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getCacheControl(version));

    return Buffer.from(fileData);
  } catch (error) {
    // If file not found, try to list as directory
    if ((error as HTTPError).statusCode === 404) {
      if (!isCached) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}. Package not yet cached.`,
        });
      }

      const listing = await getDirectoryListing(cacheBase, filepath, metadata.name, version);
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
