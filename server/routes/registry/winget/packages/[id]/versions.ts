import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import type { VersionMultipleResponse } from "../../../../../utils/winget";
import { buildPackageIndex } from "../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
    summary: "Get all versions of a WinGet package",
    description: "Retrieve all available versions for a specific WinGet package",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response",
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
                      PackageVersion: { type: "string" },
                      DefaultLocale: { type: "string" },
                      Locales: { type: "array", items: { type: "string" } },
                      Installers: { type: "array", items: { type: "string" } },
                    },
                  },
                },
                ContinuationToken: { type: "string" },
              },
            },
          },
        },
      },
      404: {
        description: "Package not found",
      },
    },
  },
});

export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");

    if (!packageId) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier is required",
      });
    }

    // Build package index
    const packageIndex = await buildPackageIndex();
    const versions = packageIndex.get(packageId);

    if (!versions) {
      throw new HTTPError({
        status: 404,
        statusText: `Package '${packageId}' not found`,
      });
    }

    const response: VersionMultipleResponse = {
      Data: Array.from(versions).map((version) => ({
        PackageVersion: version,
      })),
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 600,
    group: "registry:winget",
  },
);
