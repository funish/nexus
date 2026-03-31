import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import { getIndexDb } from "../../../../utils/winget/db";
import { packageExists } from "../../../../utils/winget/index";
import { createWinGetError } from "../../../../utils/winget/response";
import type { PackageSingleResponse } from "../../../../utils/winget/types";

defineRouteMeta({
  openAPI: {
    tags: ["Packages", "Get"],
    summary: "Get Package Metadata",
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
    ],
    responses: {
      200: {
        description: "Package metadata",
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
 * GET /packages/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get specific package
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  const db = await getIndexDb(event);

  if (!(await packageExists(db, packageId))) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  const response: PackageSingleResponse = {
    Data: {
      PackageIdentifier: packageId,
    },
  };

  return response;
});
