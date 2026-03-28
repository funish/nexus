import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { LocaleSingleResponse, LocaleSchema } from "../../../../../../../../utils/winget";
import {
  getVersionManifests,
  fetchManifestContent,
  createWinGetError,
} from "../../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet API"],
    summary: "Get specific locale for a package version",
    description: "Retrieve detailed locale information for a specific package version",
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
      {
        in: "path",
        name: "locale",
        description: "Locale code (e.g., 'en-US')",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response with locale details",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "object",
                  properties: {
                    PackageLocale: { type: "string" },
                    Publisher: { type: "string" },
                    PackageName: { type: "string" },
                    ShortDescription: { type: "string" },
                    Description: { type: "string" },
                  },
                },
              },
              required: ["Data"],
            },
          },
        },
      },
      404: {
        description: "Package, version, or locale not found",
      },
    },
  },
});

/**
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/locales/{PackageLocale}
 *
 * WinGet.RestSource API - Get specific locale
 *
 * Response: LocaleSingleResponse
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");
  const locale = getRouterParam(event, "locale");

  if (!packageId || !version || !locale) {
    return createWinGetError(
      event,
      400,
      "PackageIdentifier, PackageVersion, and PackageLocale are required",
    );
  }

  // Get all manifest files for this version
  const manifestFiles = await getVersionManifests(packageId, version);

  if (manifestFiles.length === 0) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

  // Find the specific locale manifest
  const localeFilename = `${packageId}.locale.${locale}.yaml`;
  const localeManifestPath = manifestFiles.find((path) => path.split("/").pop() === localeFilename);

  if (!localeManifestPath) {
    return createWinGetError(
      event,
      404,
      `Locale '${locale}' not found for version ${version} of package '${packageId}'`,
    );
  }

  try {
    const content = await fetchManifestContent(localeManifestPath);
    const manifest = parseYAML(content) as Record<string, any>;

    const response: LocaleSingleResponse = {
      Data: { PackageLocale: locale, ...manifest } as LocaleSchema,
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  } catch (error) {
    return createWinGetError(event, 500, `Failed to parse locale manifest: ${String(error)}`);
  }
});
