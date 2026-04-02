import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam, HTTPError } from "nitro/h3";
import semver from "semver";

import {
  type CdnFile,
  type CdnOrgListing,
  type CdnPackageListing,
  CDN_CACHE_LONG,
  CDN_CACHE_SHORT,
  CDN_NPM_REGISTRY,
  bundleEsmPackage,
  getCacheControl,
  getContentType,
  getDirectoryListing,
  isPackageCached,
  cachePackageFromTarball,
  extractFileFromTarball,
  resolveRegistryVersion,
} from "../../../utils/cdn";
import { cacheStorage } from "../../../utils/storage";

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
    // Handle org listing: @scope or @scope/ (no package name)
    const scopeOnlyMatch = path.match(/^@([^/]+)\/?$/);
    if (scopeOnlyMatch) {
      const [, scope] = scopeOnlyMatch;
      const orgRes = await fetch(`${CDN_NPM_REGISTRY}/-/org/${scope}/package`);
      if (!orgRes.ok) {
        throw new HTTPError({ status: 404, statusText: "Organization not found" });
      }
      const orgData = (await orgRes.json()) as Record<string, string>;
      const packages = Object.keys(orgData);

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", CDN_CACHE_SHORT);

      const response: CdnOrgListing = {
        name: `@${scope}`,
        packages,
      };
      return response;
    }

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
  const metadataRes = await fetch(`${CDN_NPM_REGISTRY}/${packageName}`);
  if (!metadataRes.ok) {
    throw new HTTPError({
      status: 404,
      statusText: "Package not found",
    });
  }

  const metadata = await metadataRes.json();

  // Resolve version: exact → semver range → latest dist-tag
  const resolved = resolveRegistryVersion(metadata, version);
  if (!resolved) {
    throw new HTTPError({ status: 404, statusText: "Version not found" });
  }
  const { version: resolvedVersion, versionInfo } = resolved;

  // Download tarball info
  const tarballUrl = versionInfo.dist.tarball;

  const storage = cacheStorage;
  const cacheBase = `cdn/npm/${packageName}/${resolvedVersion}`;

  // Check if entire package is already cached
  const cacheable = semver.valid(resolvedVersion) !== null;
  const isCached = await isPackageCached(cacheBase, cacheable);

  // Handle +esm bundling request
  if (filepath === "+esm") {
    // Determine entry point from package.json
    let entryFile = versionInfo.browser || versionInfo.main || versionInfo.module || "index.js";

    // Ensure package is fully cached
    if (!isCached) {
      await cachePackageFromTarball(
        tarballUrl,
        cacheBase,
        undefined,
        `npm:${packageName}@${resolvedVersion}`,
      );
    }

    // Bundle the package
    const bundledCode = await bundleEsmPackage({
      packageName,
      version: resolvedVersion,
      entryPoint: entryFile,
    });

    event.res.headers.set("Content-Type", "application/javascript; charset=utf-8");
    event.res.headers.set("Cache-Control", CDN_CACHE_LONG);
    event.res.headers.set("X-ESM-Version", resolvedVersion);

    return bundledCode;
  }

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/npm/uikit/ -> list directory contents with metadata
      // If not cached, wait for entire package to be cached
      if (!isCached) {
        await cachePackageFromTarball(
          tarballUrl,
          cacheBase,
          undefined,
          `npm:${packageName}@${resolvedVersion}`,
        );
      }

      const meta = await storage.getMeta(cacheBase);
      const fileList = (meta?.files || []) as Array<CdnFile>;

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", CDN_CACHE_SHORT);

      const response: CdnPackageListing = {
        name: metadata.name,
        version: resolvedVersion,
        path: "",
        files: fileList,
      };

      return response;
    } else {
      // /cdn/npm/uikit -> return entry file
      let entryFile = versionInfo.browser || versionInfo.main || versionInfo.module || "index.js";

      const fileData = await extractFileFromTarball(
        tarballUrl,
        entryFile,
        `${cacheBase}/${entryFile}`,
      );

      if (!isCached) {
        event.waitUntil(
          cachePackageFromTarball(
            tarballUrl,
            cacheBase,
            undefined,
            `npm:${packageName}@${resolvedVersion}`,
          ),
        );
      }

      const contentType = getContentType(entryFile);
      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set("Cache-Control", getCacheControl(resolvedVersion));

      return Buffer.from(fileData);
    }
  }

  // For non-root paths, try to get file from cache or tarball
  try {
    const fileData = await extractFileFromTarball(tarballUrl, filepath, `${cacheBase}/${filepath}`);

    if (!isCached) {
      event.waitUntil(
        cachePackageFromTarball(
          tarballUrl,
          cacheBase,
          undefined,
          `npm:${packageName}@${resolvedVersion}`,
        ),
      );
    }

    const contentType = getContentType(filepath);
    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set("Cache-Control", getCacheControl(resolvedVersion));

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

      const listing = await getDirectoryListing(
        cacheBase,
        filepath,
        metadata.name,
        resolvedVersion,
      );
      if (!listing) {
        throw new HTTPError({
          status: 404,
          statusText: `Path not found: ${filepath}`,
        });
      }

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", CDN_CACHE_SHORT);

      return listing;
    }

    throw error;
  }
});
