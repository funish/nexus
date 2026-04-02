import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";
import type { H3Event } from "nitro/h3";

import { getIndexDb } from "../../../../utils/winget/db";
import { buildVersionManifest } from "../../../../utils/winget/manifest";
import { getPackageVersions } from "../../../../utils/winget/queries";
import { createWinGetError } from "../../../../utils/winget/response";
import { compareVersion } from "../../../../utils/winget/version";

defineRouteMeta({
  openAPI: {
    tags: ["Package Manifests", "Get"],
    summary: "This returns a package manifest",
    parameters: [
      {
        in: "header",
        name: "Version",
        description: "API version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "header",
        name: "Windows-Package-Manager",
        description: "Windows Package Manager client version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "path",
        name: "PackageIdentifier",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "Version",
        description: "Filter by specific version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "Channel",
        description: "Filter by channel",
        required: false,
        schema: { type: "string", nullable: true },
      },
      {
        in: "query",
        name: "Market",
        description: "Filter by market (two-letter country code)",
        required: false,
        schema: { type: "string", nullable: true },
      },
    ],
    responses: {
      200: {
        description: "Package manifest",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "object",
                  nullable: true,
                  properties: {
                    PackageIdentifier: { type: "string" },
                    Versions: {
                      type: "array",
                      items: {
                        type: "object",
                        properties: {
                          PackageVersion: { type: "string" },
                          DefaultLocale: { type: "string" },
                          Channel: { type: "string" },
                          Locales: { type: "array", items: { type: "object" } },
                          Installers: { type: "array", items: { type: "object" } },
                        },
                        required: ["PackageVersion"],
                      },
                    },
                  },
                  required: ["PackageIdentifier"],
                },
                UnsupportedQueryParameters: { type: "array", items: { type: "string" } },
                RequiredQueryParameters: { type: "array", items: { type: "string" } },
              },
            },
          },
        },
      },
      404: { description: "Not Found" },
      default: {
        description: "An Error Occurred.",
        content: {
          "application/json": {
            schema: {
              type: "array",
              items: {
                type: "object",
                properties: {
                  ErrorCode: { type: "integer" },
                  ErrorMessage: { type: "string" },
                },
                required: ["ErrorCode", "ErrorMessage"],
              },
            },
          },
        },
      },
    },
  },
});

/**
 * GET /packageManifests/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get package manifest
 */
export default defineHandler(async (event: H3Event) => {
  const packageId = getRouterParam(event, "id");
  const query = getQuery(event);
  const filterVersion = query.Version as string | undefined;
  const filterChannel = query.Channel as string | undefined;
  const filterMarket = query.Market as string | undefined;

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  const db = await getIndexDb(event);
  let versions = getPackageVersions(db, packageId);

  if (versions.size === 0) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  if (filterVersion) {
    if (versions.has(filterVersion)) {
      versions = new Set([filterVersion]);
    } else {
      return createWinGetError(
        event,
        404,
        `Version ${filterVersion} not found for package '${packageId}'`,
      );
    }
  }

  const sortedVersions = Array.from(versions).sort((a, b) => compareVersion(b, a));

  // Build manifests for all versions in parallel
  const manifestResults = await Promise.allSettled(
    sortedVersions.map((version) => buildVersionManifest(packageId, version)),
  );

  const manifestVersions = manifestResults
    .filter((r): r is PromiseFulfilledResult<any> => r.status === "fulfilled" && r.value !== null)
    .map((r) => r.value)
    .filter((versionEntry) => {
      // Apply channel filter
      if (filterChannel && versionEntry.Channel !== filterChannel) return false;

      // Apply market filter
      if (filterMarket && versionEntry.Installers) {
        const hasMatchingInstaller = versionEntry.Installers.some((installer: any) => {
          const markets = installer.Markets;
          if (!markets) return true;
          if (markets.AllowedMarkets?.includes(filterMarket)) return true;
          if (markets.ExcludedMarkets?.includes(filterMarket)) return false;
          if (markets.AllowedMarkets) return false;
          return true;
        });
        if (!hasMatchingInstaller) return false;
      }

      return true;
    });

  return {
    Data: {
      PackageIdentifier: packageId,
      Versions: manifestVersions,
    },
    UnsupportedQueryParameters: ["FetchAllManifests"],
    RequiredQueryParameters: [],
  };
});
