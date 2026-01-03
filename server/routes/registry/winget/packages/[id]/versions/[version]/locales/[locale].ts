import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type { LocaleSingleResponse, LocaleSchema } from "../../../../../../../../utils/winget";
import { getVersionManifests, fetchManifestContent } from "../../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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
export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");
    const version = getRouterParam(event, "version");
    const locale = getRouterParam(event, "locale");

    if (!packageId || !version || !locale) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier, PackageVersion, and PackageLocale are required",
      });
    }

    // Get all manifest files for this version
    const manifestFiles = await getVersionManifests(packageId, version);

    if (manifestFiles.length === 0) {
      throw new HTTPError({
        status: 404,
        statusText: `Version ${version} of package '${packageId}' not found`,
      });
    }

    // Find the specific locale manifest
    const localeFilename = `${packageId}.locale.${locale}.yaml`;
    const localeManifestPath = manifestFiles.find(
      (path) => path.split("/").pop() === localeFilename,
    );

    if (!localeManifestPath) {
      throw new HTTPError({
        status: 404,
        statusText: `Locale '${locale}' not found for version ${version} of package '${packageId}'`,
      });
    }

    try {
      const content = await fetchManifestContent(localeManifestPath);
      const manifest = parseYAML(content) as Record<string, any>;

      const localeData: LocaleSchema = {
        PackageLocale: locale,
        Publisher: manifest.Publisher,
        PackageName: manifest.PackageName,
        ShortDescription: manifest.ShortDescription,
        Description: manifest.Description,
      };

      const response: LocaleSingleResponse = {
        Data: localeData,
      };

      event.res.headers.set("Content-Type", "application/json");

      return response;
    } catch (error) {
      throw new HTTPError({
        status: 500,
        statusText: `Failed to parse locale manifest: ${String(error)}`,
      });
    }
  },
  {
    maxAge: 3600,
  },
);
