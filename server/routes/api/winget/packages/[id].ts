import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { PackageSingleResponse } from "../../../../utils/winget";
import { buildPackageIndex, createWinGetError } from "../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Packages", "Get"],
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
                Data: {
                  type: "object",
                  properties: {
                    PackageIdentifier: { type: "string" },
                  },
                  required: ["PackageIdentifier"],
                },
              },
              required: ["Data"],
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
 * GET /api/winget/packages/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get specific package
 *
 * Response: PackageSingleResponse
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  // Build package index
  const packageIndex = await buildPackageIndex(event);
  const versions = packageIndex.get(packageId);

  if (!versions) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  const response: PackageSingleResponse = {
    Data: {
      PackageIdentifier: packageId,
    },
  };

  event.res.headers.set("Content-Type", "application/json");

  return response;
});
