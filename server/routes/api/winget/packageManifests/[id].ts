import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";
import type { H3Event } from "nitro/h3";

import { getIndexDb } from "../../../../utils/winget/db";
import { getPackageVersions } from "../../../../utils/winget/index";
import { getVersionManifests, fetchManifestContent } from "../../../../utils/winget/manifest";
import { createWinGetError } from "../../../../utils/winget/response";
import type { VersionManifest } from "../../../../utils/winget/types";
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
  let versions = await getPackageVersions(db, packageId);

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
  const manifestVersions: VersionManifest[] = [];

  await Promise.allSettled(
    sortedVersions.map(async (version) => {
      const manifestFiles = await getVersionManifests(packageId, version);
      if (manifestFiles.length === 0) return;

      // Fetch all manifest files in parallel
      const fetched = await Promise.allSettled(
        manifestFiles.map(async (manifestPath) => {
          const content = await fetchManifestContent(manifestPath);
          return {
            filename: manifestPath.split("/").pop()!,
            manifest: parseYAML(content) as Record<string, any>,
          };
        }),
      );

      const versionEntry: VersionManifest = { PackageVersion: version };

      for (const result of fetched) {
        if (result.status !== "fulfilled") continue;
        const { filename, manifest } = result.value;

        if (filename === `${packageId}.yaml`) {
          versionEntry.DefaultLocale = manifest.DefaultLocale;
          versionEntry.Channel = manifest.Channel;
          const hasLocaleData = Boolean(
            manifest.PackageLocale || manifest.Publisher || manifest.PackageName,
          );
          const dl = manifest.DefaultLocale || manifest.PackageLocale;
          const hasDefaultLocaleFile = dl
            ? manifestFiles.some((p) => p.includes(`.locale.${dl}.yaml`))
            : false;
          if (hasLocaleData && !hasDefaultLocaleFile) {
            if (!versionEntry.Locales) versionEntry.Locales = [];
            versionEntry.Locales.unshift({
              PackageLocale: dl || manifest.PackageLocale,
              ...manifest,
            } as Record<string, any>);
          }
        } else if (filename.match(/\.installer\.yaml$/)) {
          if (manifest.Installers && Array.isArray(manifest.Installers)) {
            versionEntry.Installers = manifest.Installers.map((inst: Record<string, any>) => ({
              ...manifest,
              ...inst,
            }));
          }
        } else if (filename.match(/\.locale\.([^.]+)\.yaml$/)) {
          if (!versionEntry.Locales) versionEntry.Locales = [];
          versionEntry.Locales.push(manifest);
        }
      }

      if (filterChannel && versionEntry.Channel !== filterChannel) return;

      if (filterMarket && versionEntry.Installers) {
        const hasMatchingInstaller = versionEntry.Installers.some((installer: any) => {
          const markets = installer.Markets;
          if (!markets) return true;
          if (markets.AllowedMarkets?.includes(filterMarket)) return true;
          if (markets.ExcludedMarkets?.includes(filterMarket)) return false;
          if (markets.AllowedMarkets) return false;
          return true;
        });
        if (!hasMatchingInstaller) return;
      }

      manifestVersions.push(versionEntry);
    }),
  );

  return {
    Data: {
      PackageIdentifier: packageId,
      Versions: manifestVersions,
    },
    UnsupportedQueryParameters: ["FetchAllManifests"],
    RequiredQueryParameters: [],
  };
});
