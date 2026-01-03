import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import type { PackageSingleResponse } from "../../../../utils/winget";
import { buildPackageIndex } from "../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
    summary: "Get specific WinGet package",
    description: "Retrieve details of a specific WinGet package by its identifier",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier (e.g., 'Microsoft.VisualStudioCode')",
        required: true,
        schema: {
          type: "string",
        },
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
                PackageIdentifier: { type: "string" },
                Versions: { type: "array", items: { type: "string" } },
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

/**
 * GET /registry/winget/packages/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get specific package
 *
 * Response: PackageSingleResponse
 */
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
    const packageIndex = await buildPackageIndex(event);
    const versions = packageIndex.get(packageId);

    if (!versions) {
      throw new HTTPError({
        status: 404,
        statusText: `Package '${packageId}' not found`,
      });
    }

    const response: PackageSingleResponse = {
      Data: {
        PackageIdentifier: packageId,
      },
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 600,
  },
);
