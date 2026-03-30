import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";

import { getIndexDb } from "../../../../../utils/winget/db";
import { getPackageVersions } from "../../../../../utils/winget/index";
import { constructManifestPath, fetchManifestContent } from "../../../../../utils/winget/manifest";
import type { VersionMultipleResponse, VersionSchema } from "../../../../../utils/winget/types";
import { createWinGetError, compareVersion } from "../../../../../utils/winget/utils";

defineRouteMeta({
  openAPI: {
    tags: ["Versions", "Get"],
    summary: "Get Version Metadata",
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
        description: "Version list",
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
                      Channel: { type: "string" },
                    },
                    required: ["PackageVersion", "DefaultLocale"],
                  },
                },
                ContinuationToken: { type: "string" },
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

const PAGE_SIZE = 25;

/**
 * GET /packages/{PackageIdentifier}/versions
 *
 * WinGet.RestSource API - Get all versions
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  const query = getQuery(event);
  const continuationToken = query.ContinuationToken as string | undefined;

  const db = await getIndexDb(event);
  const versions = await getPackageVersions(db, packageId);

  if (versions.size === 0) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  const sortedVersions = Array.from(versions).sort((a, b) => compareVersion(b, a));

  // Fetch DefaultLocale from the latest version's manifest (all versions usually share the same DefaultLocale)
  let defaultLocale = "en-US";
  if (sortedVersions.length > 0) {
    try {
      const mainPath = constructManifestPath(packageId, sortedVersions[0]!, "main");
      const content = await fetchManifestContent(mainPath);
      defaultLocale = (parseYAML(content) as Record<string, any>).DefaultLocale || "en-US";
    } catch {
      // Keep fallback
    }
  }

  let startIndex = 0;
  if (continuationToken) {
    try {
      startIndex = parseInt(Buffer.from(continuationToken, "base64").toString(), 10);
    } catch {
      startIndex = 0;
    }
  }

  const endIndex = startIndex + PAGE_SIZE;
  const paginatedVersions = sortedVersions.slice(startIndex, endIndex);

  const versionData: VersionSchema[] = paginatedVersions.map((version) => ({
    PackageVersion: version,
    DefaultLocale: defaultLocale,
  }));

  const response: VersionMultipleResponse = {
    Data: versionData,
  };

  if (endIndex < sortedVersions.length) {
    response.ContinuationToken = Buffer.from(endIndex.toString()).toString("base64");
  }

  return response;
});
