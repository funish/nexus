import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, readBody } from "nitro/h3";

import { getIndexDb } from "../../../utils/winget/db";
import {
  buildSearchIndex,
  getSearchIndex,
  persistSearchIndex,
  searchPackages,
} from "../../../utils/winget/search";
import { encodeContinuationToken } from "../../../utils/winget/token";
import type {
  WinGetManifestSearchRequest,
  WinGetManifestSearchResult,
  WinGetMatchType,
} from "../../../utils/winget/types";

defineRouteMeta({
  openAPI: {
    tags: ["Package Manifests"],
    summary: "This retrieves package manifests for a given search request.",
    description:
      "Supports both GET (query parameters: query, matchType, maximumResults) and POST (JSON request body with Inclusions/Filters).",
    parameters: [
      {
        in: "query",
        name: "query",
        description: "Search keyword (GET only)",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "matchType",
        description:
          "Match type: Exact, CaseInsensitive, StartsWith, Substring, Wildcard, Fuzzy, FuzzySubstring (GET only)",
        required: false,
        schema: {
          type: "string",
          enum: [
            "Exact",
            "CaseInsensitive",
            "StartsWith",
            "Substring",
            "Wildcard",
            "Fuzzy",
            "FuzzySubstring",
          ],
        },
      },
      {
        in: "query",
        name: "maximumResults",
        description: "Maximum number of results to return (GET only)",
        required: false,
        schema: { type: "number" },
      },
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
        in: "header",
        name: "ContinuationToken",
        description: "Pagination token",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Search results",
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
                      PackageName: { type: "string" },
                      Publisher: { type: "string" },
                      Versions: {
                        type: "array",
                        items: {
                          type: "object",
                          properties: {
                            PackageVersion: { type: "string" },
                            Channel: { type: "string" },
                            PackageFamilyNames: { type: "array", items: { type: "string" } },
                            ProductCodes: { type: "array", items: { type: "string" } },
                            AppsAndFeaturesEntryVersions: {
                              type: "array",
                              items: { type: "string" },
                            },
                            UpgradeCodes: { type: "array", items: { type: "string" } },
                          },
                          required: ["PackageVersion"],
                        },
                      },
                    },
                    required: ["PackageIdentifier", "PackageName", "Publisher"],
                  },
                },
                ContinuationToken: { type: "string" },
                RequiredPackageMatchFields: { type: "array", items: { type: "string" } },
                UnsupportedPackageMatchFields: { type: "array", items: { type: "string" } },
              },
            },
          },
        },
      },
      204: { description: "No results were found." },
      400: { description: "Bad Request" },
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
 * POST /manifestSearch
 *
 * WinGet.RestSource API - Search packages
 *
 * Supports both GET and POST methods for compatibility:
 * - POST: JSON request body (official format with Inclusions/Filters)
 * - GET: Query parameters (compatibility mode)
 */
export default defineHandler(async (event) => {
  let searchRequest: WinGetManifestSearchRequest;

  if (event.req.method === "POST") {
    const body = (await readBody(event)) as WinGetManifestSearchRequest;
    searchRequest = {
      MaximumResults: body.MaximumResults,
      FetchAllManifests: body.FetchAllManifests,
      Query: body.Query,
      Inclusions: body.Inclusions,
      Filters: body.Filters,
    };
  } else {
    const query = getQuery(event);
    searchRequest = {
      MaximumResults: query.maximumResults
        ? parseInt(query.maximumResults as string, 10)
        : undefined,
      FetchAllManifests: query.fetchAllManifests === "true",
      Query: query.query
        ? {
            KeyWord: query.query as string,
            MatchType: (query.matchType as WinGetMatchType) || "CaseInsensitive",
          }
        : undefined,
    };
  }

  let searchIndex = await getSearchIndex();
  if (!searchIndex) {
    const db = await getIndexDb(event);
    searchIndex = buildSearchIndex(db);
    await persistSearchIndex(searchIndex);
  }

  // ContinuationToken from header (spec defines it as a header parameter)
  const headerToken = event.req.headers.get("continuationtoken") || undefined;

  const { results, hasMore, offset } = searchPackages({
    keyword: searchRequest.Query?.KeyWord,
    matchType: searchRequest.Query?.MatchType,
    maximumResults: searchRequest.MaximumResults,
    continuationToken: headerToken,
    inclusions: searchRequest.Inclusions,
    filters: searchRequest.Filters,
    searchIndex,
  });

  const response: WinGetManifestSearchResult = {
    Data: results,
    RequiredPackageMatchFields: ["PackageIdentifier"],
    UnsupportedPackageMatchFields: ["Market", "HasInstallerType"],
  };

  if (hasMore) {
    response.ContinuationToken = encodeContinuationToken(offset + results.length);
  }

  event.res.headers.set("Cache-Control", "public, max-age=300");

  if (results.length === 0) {
    event.res.status = 204;
    return null;
  }

  return response;
});
