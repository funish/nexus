import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, readBody } from "nitro/h3";

import type { ManifestSearchRequest, ManifestSearchResult, MatchType } from "../../../utils/winget";
import { getIndexDb, searchPackages } from "../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Package Manifests"],
    summary: "Search WinGet packages",
    description: "Search for WinGet packages by keyword with various match types",
    parameters: [
      {
        in: "query",
        name: "query",
        description: "Search keyword",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "matchType",
        description:
          "Match type (Exact, CaseInsensitive, StartsWith, Substring, Wildcard, Fuzzy, FuzzySubstring)",
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
        description: "Maximum number of results to return",
        required: false,
        schema: { type: "number" },
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
                          },
                        },
                      },
                    },
                  },
                },
                RequiredPackageMatchFields: {
                  type: "array",
                  items: { type: "string" },
                },
                UnsupportedPackageMatchFields: {
                  type: "array",
                  items: { type: "string" },
                },
              },
            },
          },
        },
      },
    },
  },
});

/**
 * GET/POST /api/winget/manifestSearch
 *
 * WinGet.RestSource API - Search packages
 *
 * Supports both GET and POST methods for compatibility:
 * - GET: Query parameters (query, matchType, maximumResults)
 * - POST: JSON request body (official format with Inclusions/Filters)
 *
 * Request body (POST):
 * {
 *   "MaximumResults": 10,
 *   "FetchAllManifests": false,
 *   "Query": { "KeyWord": "Adobe", "MatchType": "CaseInsensitive" },
 *   "Inclusions": [{ "PackageMatchField": "PackageName", "RequestMatch": { "KeyWord": "Microsoft", "MatchType": "CaseInsensitive" } }],
 *   "Filters": [{ "PackageMatchField": "Tag", "RequestMatch": { "KeyWord": "preview", "MatchType": "CaseInsensitive" } }]
 * }
 */
export default defineHandler(async (event) => {
  let searchRequest: ManifestSearchRequest;

  if (event.req.method === "POST") {
    const body = (await readBody(event)) as ManifestSearchRequest;
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
            MatchType: (query.matchType as MatchType) || "CaseInsensitive",
          }
        : undefined,
    };
  }

  const db = await getIndexDb(event);

  const results = searchPackages(db, {
    keyword: searchRequest.Query?.KeyWord,
    matchType: searchRequest.Query?.MatchType,
    maximumResults: searchRequest.MaximumResults,
    inclusions: searchRequest.Inclusions,
    filters: searchRequest.Filters,
  });

  const response: ManifestSearchResult = {
    Data: results,
    RequiredPackageMatchFields: ["PackageIdentifier"],
    UnsupportedPackageMatchFields: ["Market", "HasInstallerType"],
  };

  event.res.headers.set("Content-Type", "application/json");
  event.res.headers.set("Cache-Control", "public, max-age=300"); // 5 minutes

  return response;
});
