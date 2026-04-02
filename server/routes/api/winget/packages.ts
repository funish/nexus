import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery } from "nitro/h3";

import { WINGET_PACKAGES_PAGE_SIZE } from "../../../utils/winget/constants";
import { getPackageIndex } from "../../../utils/winget/queries";
import { decodeContinuationToken, encodeContinuationToken } from "../../../utils/winget/token";
import type { WinGetPackageMultipleResponse } from "../../../utils/winget/types";

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

  const packageIndex = await getPackageIndex(event);
  const packages = Array.from(packageIndex.keys()).sort();

  const startIndex = decodeContinuationToken(continuationToken);
  const endIndex = startIndex + WINGET_PACKAGES_PAGE_SIZE;
  const paginatedPackages = packages.slice(startIndex, endIndex);

  const response: WinGetPackageMultipleResponse = {
    Data: paginatedPackages.map((id) => ({ PackageIdentifier: id })),
  };

  if (endIndex < packages.length) {
    response.ContinuationToken = encodeContinuationToken(endIndex);
  }
  return response;
});
