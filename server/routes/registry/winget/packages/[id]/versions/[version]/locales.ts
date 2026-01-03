import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type { LocaleMultipleResponse, LocaleSchema } from "../../../../../../../utils/winget";
import { getVersionManifests, fetchManifestContent } from "../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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
export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");
    const version = getRouterParam(event, "version");

    if (!packageId || !version) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier and PackageVersion are required",
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

    // Find all locale manifests
    const locales: LocaleSchema[] = [];

    for (const manifestPath of manifestFiles) {
      const filename = manifestPath.split("/").pop()!;

      // Match locale files: {PackageId}.locale.{locale}.yaml
      const localeMatch = filename.match(/\.locale\.([^.]+)\.yaml$/);

      if (localeMatch && localeMatch[1]) {
        const locale = localeMatch[1];

        try {
          const content = await fetchManifestContent(manifestPath);
          const manifest = parseYAML(content) as Record<string, any>;

          locales.push({
            PackageLocale: locale,
            Publisher: manifest.Publisher,
            PackageName: manifest.PackageName,
            ShortDescription: manifest.ShortDescription,
            Description: manifest.Description,
          });
        } catch {
          // If parsing fails, add minimal locale info
          locales.push({
            PackageLocale: locale,
          });
        }
      }
    }

    const response: LocaleMultipleResponse = {
      Data: locales,
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 3600,
  },
);
