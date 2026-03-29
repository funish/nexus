import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { LocaleMultipleResponse, LocaleSchema } from "../../../../../../../utils/winget";
import {
  getVersionManifests,
  getDefaultLocaleManifestPath,
  fetchManifestContent,
  createWinGetError,
} from "../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Locale", "Get"],
    summary: "Get all locales for a package version",
    description: "Retrieve all available locales for a specific package version",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "path",
        name: "version",
        description: "Package version",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response with locales list",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "array",
                  items: {
                    type: "object",
                    properties: {
                      PackageLocale: { type: "string" },
                      Publisher: { type: "string" },
                      PackageName: { type: "string" },
                      ShortDescription: { type: "string" },
                      Description: { type: "string" },
                    },
                    required: ["PackageLocale"],
                  },
                },
                ContinuationToken: { type: "string" },
              },
              required: ["Data"],
            },
          },
        },
      },
      404: {
        description: "Package or version not found",
      },
    },
  },
});

/**
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/locales
 *
 * WinGet.RestSource API - Get all locales for a version
 *
 * Response: LocaleMultipleResponse
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");

  if (!packageId || !version) {
    return createWinGetError(event, 400, "PackageIdentifier and PackageVersion are required");
  }

  // Get all manifest files for this version
  const manifestFiles = getVersionManifests(packageId, version);

  if (manifestFiles.length === 0) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

  // Find all locale manifests
  const locales: LocaleSchema[] = [];

  // Read version manifest to get DefaultLocale
  const versionManifestPath = manifestFiles.find(
    (path) => path.split("/").pop() === `${packageId}.yaml`,
  );

  let defaultLocale = "en-US";
  if (versionManifestPath) {
    try {
      const content = await fetchManifestContent(versionManifestPath);
      const manifest = parseYAML(content) as Record<string, any>;
      if (manifest.DefaultLocale) {
        defaultLocale = manifest.DefaultLocale;
      }
    } catch {
      // keep fallback
    }
  }

  const localePath = getDefaultLocaleManifestPath(packageId, version, defaultLocale);

  try {
    const content = await fetchManifestContent(localePath);
    const manifest = parseYAML(content) as Record<string, any>;

    locales.push({
      PackageLocale: defaultLocale,
      ...manifest,
    });
  } catch {
    locales.push({ PackageLocale: defaultLocale });
  }

  const response: LocaleMultipleResponse = {
    Data: locales,
  };

  return response;
});
