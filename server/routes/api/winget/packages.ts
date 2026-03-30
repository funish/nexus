import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery } from "nitro/h3";

import { buildPackageIndex } from "../../../utils/winget/index";
import type { PackageMultipleResponse } from "../../../utils/winget/types";

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
        in: "query",
        name: "ContinuationToken",
        description: "Pagination token",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Package list",
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
                      PackageIdentifier: { type: "string" },
                    },
                    required: ["PackageIdentifier"],
                  },
                },
                ContinuationToken: { type: "string" },
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
 * GET /packages
 *
 * WinGet.RestSource API - Get all packages
 */
export default defineHandler(async (event) => {
  const query = getQuery(event);
  const continuationToken = query.ContinuationToken as string | undefined;

  const packageIndex = await buildPackageIndex(event);

  const packages = Array.from(packageIndex.keys()).sort();

  const pageSize = 100;
  let startIndex = 0;

  if (continuationToken) {
    try {
      startIndex = parseInt(Buffer.from(continuationToken, "base64").toString(), 10);
    } catch {
      startIndex = 0;
    }
  }

  const endIndex = startIndex + pageSize;
  const paginatedPackages = packages.slice(startIndex, endIndex);

  const response: PackageMultipleResponse = {
    Data: paginatedPackages.map((id) => ({ PackageIdentifier: id })),
  };

  if (endIndex < packages.length) {
    response.ContinuationToken = Buffer.from(endIndex.toString()).toString("base64");
  }
  return response;
});
