import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, readBody } from "nitro/h3";
import type {
  MatchType,
  ManifestSearchRequest,
  ManifestSearchResult,
  ManifestSearchResponse,
} from "../../../utils/winget";
import { buildPackageIndex } from "../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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
 * GET/POST /registry/winget/manifestSearch
 *
 * WinGet.RestSource API - Search packages
 *
 * Supports both GET and POST methods for compatibility:
 * - GET: Query parameters (query, matchType, maximumResults)
 * - POST: JSON request body (official format)
 *
 * Query parameters (GET):
 * - query: Search keyword
 * - matchType: Exact | CaseInsensitive | StartsWith | Substring | Wildcard | Fuzzy | FuzzySubstring
 * - maximumResults: Maximum number of results to return
 *
 * Request body (POST):
 * {
 *   "MaximumResults": 10,
 *   "FetchAllManifests": false,
 *   "Query": {
 *     "KeyWord": "Adobe",
 *     "MatchType": "CaseInsensitive"
 *   },
 *   "Inclusions": [...],
 *   "Filters": [...]
 * }
 *
 * Response: ManifestSearchResult
 */
export default defineHandler(async (event) => {
  let searchRequest: ManifestSearchRequest;

  // Handle both GET and POST methods
  if (event.req.method === "POST") {
    // POST: Parse from request body (official format)
    const body = (await readBody(event)) as ManifestSearchRequest;
    searchRequest = {
      MaximumResults: body.MaximumResults,
      FetchAllManifests: body.FetchAllManifests,
      Query: body.Query,
    };
  } else {
    // GET (default): Parse from query parameters
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

  // Build package index
  const packageIndex = await buildPackageIndex(event);
  const results: ManifestSearchResponse[] = [];

  const { MaximumResults, Query } = searchRequest;

  for (const [packageId, versions] of packageIndex.entries()) {
    // Apply MaximumResults limit
    if (MaximumResults && results.length >= MaximumResults) {
      break;
    }

    // Check query match (PackageIdentifier)
    if (Query && Query.KeyWord) {
      const queryMatch = matchText(packageId, Query.KeyWord, Query.MatchType || "CaseInsensitive");
      if (!queryMatch) {
        continue;
      }
    }

    // Package matched all criteria
    results.push({
      PackageIdentifier: packageId,
      Versions: Array.from(versions)
        .sort()
        .reverse()
        .slice(0, 10) // Limit to 10 versions
        .map((version) => ({
          PackageVersion: version,
        })),
    });
  }

  const response: ManifestSearchResult = {
    Data: results,
    RequiredPackageMatchFields: ["PackageIdentifier"],
    UnsupportedPackageMatchFields: ["Market", "NormalizedPackageNameAndPublisher"],
  };

  event.res.headers.set("Content-Type", "application/json");
  event.res.headers.set("Cache-Control", "public, max-age=300"); // 5 minutes

  return response;
});

/**
 * Match text based on match type
 */
function matchText(text: string, keyword: string, matchType: MatchType): boolean {
  if (!keyword) return true;

  const normalizedText = text.toLowerCase();
  const normalizedKeyword = keyword.toLowerCase();

  switch (matchType) {
    case "Exact":
      return normalizedText === normalizedKeyword;
    case "CaseInsensitive":
      return normalizedText.includes(normalizedKeyword);
    case "StartsWith":
      return normalizedText.startsWith(normalizedKeyword);
    case "Substring":
      return normalizedText.includes(normalizedKeyword);
    case "Wildcard":
      // Simple wildcard support (* matches any characters)
      const wildcardPattern = normalizedKeyword.replace(/\*/g, ".*");
      const regex = new RegExp(`^${wildcardPattern}$`, "i");
      return regex.test(text);
    case "Fuzzy":
      // Simple fuzzy match: keyword characters appear in order
      let keywordIndex = 0;
      for (const char of normalizedText) {
        if (keywordIndex < normalizedKeyword.length && char === normalizedKeyword[keywordIndex]) {
          keywordIndex++;
        }
      }
      return keywordIndex === normalizedKeyword.length;
    case "FuzzySubstring":
      // Fuzzy match on any word in text
      const words = normalizedText.split(/\s+/);
      return words.some((word) => {
        let keywordIndex = 0;
        for (const char of word) {
          if (keywordIndex < normalizedKeyword.length && char === normalizedKeyword[keywordIndex]) {
            keywordIndex++;
          }
        }
        return keywordIndex === normalizedKeyword.length;
      });
    default:
      return false;
  }
}
