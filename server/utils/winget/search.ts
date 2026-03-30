import { Database } from "bun:sqlite";

import { distance } from "fastest-levenshtein";

import type {
  MatchType,
  ManifestSearchResponse,
  ManifestSearchVersionResponse,
  PackageMatchField,
  SearchRequestPackageMatchFilter,
} from "./types";
import { compareVersion } from "./utils";

// Text matching

/**
 * Convert MatchType + keyword to SQL LIKE pattern
 */
export function toSqlPattern(keyword: string, matchType: MatchType): string {
  const kw = keyword.toLowerCase();
  switch (matchType) {
    case "Exact":
      return kw;
    case "CaseInsensitive":
    case "Substring":
      return `%${kw}%`;
    case "StartsWith":
      return `${kw}%`;
    case "Wildcard":
      return kw.replace(/\*/g, "%");
    default:
      return `%${kw}%`;
  }
}

/**
 * Get the SQL table/column for a PackageMatchField
 */
export function getFieldTable(field: PackageMatchField): string | null {
  switch (field) {
    case "PackageIdentifier":
      return "i.id";
    case "PackageName":
      return "n.name";
    case "Moniker":
      return "mk.moniker";
    case "Command":
      return null;
    case "Tag":
      return null;
    case "PackageFamilyName":
      return null;
    case "ProductCode":
      return null;
    case "UpgradeCode":
      return null;
    case "NormalizedPackageNameAndPublisher":
      return null;
    case "Market":
      return null;
    case "HasInstallerType":
      return null;
    default:
      return null;
  }
}

/**
 * Build EXISTS/NOT EXISTS subquery condition for map-table-based PackageMatchFields
 */
export function buildSubqueryCondition(
  field: PackageMatchField,
  paramIdx: number,
  negate: boolean,
): string {
  const not = negate ? "NOT " : "";
  switch (field) {
    case "Command":
      return `${not}EXISTS (SELECT 1 FROM commands_map cm JOIN commands c ON c.rowid = cm.command WHERE cm.manifest = m.rowid AND c.command LIKE ?${paramIdx} COLLATE NOCASE)`;
    case "Tag":
      return `${not}EXISTS (SELECT 1 FROM tags_map tm JOIN tags t ON t.rowid = tm.tag WHERE tm.manifest = m.rowid AND t.tag LIKE ?${paramIdx} COLLATE NOCASE)`;
    case "PackageFamilyName":
      return `${not}EXISTS (SELECT 1 FROM pfns_map pm JOIN pfns p ON p.rowid = pm.pfn WHERE pm.manifest = m.rowid AND p.pfn LIKE ?${paramIdx} COLLATE NOCASE)`;
    case "ProductCode":
      return `${not}EXISTS (SELECT 1 FROM productcodes_map pcm JOIN productcodes pc ON pc.rowid = pcm.productcode WHERE pcm.manifest = m.rowid AND pc.productcode LIKE ?${paramIdx} COLLATE NOCASE)`;
    case "UpgradeCode":
      return `${not}EXISTS (SELECT 1 FROM upgradecodes_map ucm JOIN upgradecodes uc ON uc.rowid = ucm.upgradecode WHERE ucm.manifest = m.rowid AND uc.upgradecode LIKE ?${paramIdx} COLLATE NOCASE)`;
    case "NormalizedPackageNameAndPublisher":
      return `(${not}EXISTS (SELECT 1 FROM norm_names_map nnm JOIN norm_names nn ON nn.rowid = nnm.norm_name WHERE nnm.manifest = m.rowid AND nn.norm_name LIKE ?${paramIdx} COLLATE NOCASE) ${negate ? "AND" : "OR"} ${not}EXISTS (SELECT 1 FROM norm_publishers_map npm JOIN norm_publishers np ON np.rowid = npm.norm_publisher WHERE npm.manifest = m.rowid AND np.norm_publisher LIKE ?${paramIdx} COLLATE NOCASE))`;
    default:
      return "1=1";
  }
}

// Main search function

/**
 * Search packages using index.db SQL queries.
 * Uses LEFT JOIN for monikers and channels to avoid excluding
 * packages that have no moniker or channel.
 */
export function searchPackages(
  db: Database,
  options: {
    keyword?: string;
    matchType?: MatchType;
    maximumResults?: number;
    continuationToken?: string;
    inclusions?: SearchRequestPackageMatchFilter[];
    filters?: SearchRequestPackageMatchFilter[];
  },
): { results: ManifestSearchResponse[]; hasMore: boolean } {
  const { keyword, matchType, maximumResults, continuationToken, inclusions, filters } = options;
  const isFuzzy = matchType === "Fuzzy" || matchType === "FuzzySubstring";

  // Parse continuationToken as base64 encoded offset
  let offset = 0;
  if (continuationToken) {
    try {
      offset = parseInt(Buffer.from(continuationToken, "base64").toString(), 10);
    } catch {
      offset = 0;
    }
  }

  // Build WHERE clauses
  const conditions: string[] = [];
  const params: string[] = [];

  // Main keyword search across ids, names, monikers
  if (keyword) {
    if (isFuzzy) {
      const words = keyword.toLowerCase().split(/\s+/).filter(Boolean);
      const trigramParts: string[] = [];
      for (const w of words) {
        const prefix = w.slice(0, Math.min(3, w.length));
        const idx = params.length + 1;
        trigramParts.push(
          `(i.id LIKE ?${idx} COLLATE NOCASE OR n.name LIKE ?${idx} COLLATE NOCASE OR mk.moniker LIKE ?${idx} COLLATE NOCASE)`,
        );
        params.push(`%${prefix}%`);
      }
      conditions.push(`(${trigramParts.join(" OR ")})`);
    } else {
      const pattern = toSqlPattern(keyword, matchType || "CaseInsensitive");
      conditions.push(
        `(i.id LIKE ?1 COLLATE NOCASE OR n.name LIKE ?1 COLLATE NOCASE OR mk.moniker LIKE ?1 COLLATE NOCASE)`,
      );
      params.push(pattern);
    }
  }

  // Inclusions (AND semantics — all must match)
  if (inclusions) {
    for (const inc of inclusions) {
      if (!inc.RequestMatch?.KeyWord || !inc.PackageMatchField) continue;

      const column = getFieldTable(inc.PackageMatchField);
      const incMatchType = inc.RequestMatch.MatchType;
      const isIncFuzzy = incMatchType === "Fuzzy" || incMatchType === "FuzzySubstring";

      if (isIncFuzzy) {
        const words = inc.RequestMatch.KeyWord.toLowerCase().split(/\s+/).filter(Boolean);
        for (const w of words) {
          const prefix = w.slice(0, Math.min(3, w.length));
          if (column) {
            const idx = params.length + 1;
            conditions.push(`${column} LIKE ?${idx} COLLATE NOCASE`);
          } else {
            conditions.push(
              buildSubqueryCondition(inc.PackageMatchField, params.length + 1, false),
            );
          }
          params.push(`%${prefix}%`);
        }
        continue;
      }

      const pattern = toSqlPattern(inc.RequestMatch.KeyWord, incMatchType || "CaseInsensitive");

      if (!column) {
        conditions.push(buildSubqueryCondition(inc.PackageMatchField, params.length + 1, false));
      } else {
        conditions.push(`${column} LIKE ?${params.length + 1} COLLATE NOCASE`);
      }
      params.push(pattern);
    }
  }

  // Filters (NOT semantics — must not match)
  if (filters) {
    for (const f of filters) {
      if (!f.RequestMatch?.KeyWord || !f.PackageMatchField) continue;

      const column = getFieldTable(f.PackageMatchField);
      const pattern = toSqlPattern(
        f.RequestMatch.KeyWord,
        f.RequestMatch.MatchType || "CaseInsensitive",
      );

      if (!column) {
        conditions.push(buildSubqueryCondition(f.PackageMatchField, params.length + 1, true));
      } else {
        conditions.push(`${column} NOT LIKE ?${params.length + 1} COLLATE NOCASE`);
      }
      params.push(pattern);
    }
  }

  const whereClause = conditions.length > 0 ? `WHERE ${conditions.join(" AND ")}` : "";
  // Fetch more rows than maximumResults because DISTINCT rows group into fewer packages.
  // A single package may have many versions/manifests.
  const fetchCount = maximumResults ? Math.max(maximumResults * 10, 500) : 500;
  const limitClause = `LIMIT ${fetchCount} OFFSET ${offset}`;

  // Use \x1E (Record Separator) as delimiter to avoid conflicts with field values
  const DELIM = "\x1E";

  // LEFT JOIN for monikers and channels to avoid excluding packages without them
  const sql = `
    SELECT DISTINCT i.id, n.name, v.version, ch.channel, np.norm_publisher,
      (SELECT GROUP_CONCAT(p.pfn, '${DELIM}') FROM pfns_map pm JOIN pfns p ON p.rowid = pm.pfn WHERE pm.manifest = m.rowid) as pfns,
      (SELECT GROUP_CONCAT(pc.productcode, '${DELIM}') FROM productcodes_map pcm JOIN productcodes pc ON pc.rowid = pcm.productcode WHERE pcm.manifest = m.rowid) as productcodes,
      (SELECT GROUP_CONCAT(uc.upgradecode, '${DELIM}') FROM upgradecodes_map ucm JOIN upgradecodes uc ON uc.rowid = ucm.upgradecode WHERE ucm.manifest = m.rowid) as upgradecodes
    FROM manifest m
    JOIN ids i ON m.id = i.rowid
    JOIN names n ON m.name = n.rowid
    JOIN versions v ON m.version = v.rowid
    LEFT JOIN monikers mk ON m.moniker = mk.rowid
    LEFT JOIN channels ch ON m.channel = ch.rowid
    LEFT JOIN norm_publishers_map npm ON npm.manifest = m.rowid
    LEFT JOIN norm_publishers np ON np.rowid = npm.norm_publisher
    ${whereClause}
    ${limitClause}
  `;

  const rows = db
    .query<
      {
        id: string;
        name: string;
        version: string;
        channel: string;
        norm_publisher: string | null;
        pfns: string | null;
        productcodes: string | null;
        upgradecodes: string | null;
      },
      string[]
    >(sql)
    .all(...params);

  // Group by package id
  const packageMap = new Map<
    string,
    { name: string; publisher: string; versions: ManifestSearchVersionResponse[] }
  >();

  for (const row of rows) {
    let entry = packageMap.get(row.id);
    if (!entry) {
      const publisher = row.norm_publisher || row.id.split(".")[0] || "";
      entry = { name: row.name, publisher, versions: [] };
      packageMap.set(row.id, entry);
    }
    if (row.norm_publisher && !entry.publisher) {
      entry.publisher = row.norm_publisher;
    }
    entry.versions.push({
      PackageVersion: row.version,
      ...(row.channel && { Channel: row.channel }),
      ...(row.pfns && {
        PackageFamilyNames: [...new Set(row.pfns.split("\x1E").filter(Boolean))],
      }),
      ...(row.productcodes && {
        ProductCodes: [...new Set(row.productcodes.split("\x1E").filter(Boolean))],
      }),
      ...(row.upgradecodes && {
        UpgradeCodes: [...new Set(row.upgradecodes.split("\x1E").filter(Boolean))],
      }),
    });
  }

  // Build response, sort versions descending per package
  const results: ManifestSearchResponse[] = [];
  for (const [id, data] of packageMap.entries()) {
    data.versions.sort((a, b) => compareVersion(b.PackageVersion, a.PackageVersion));
    results.push({
      PackageIdentifier: id,
      PackageName: data.name,
      Publisher: data.publisher,
      Versions: data.versions,
    });
  }

  // Fuzzy scoring and re-ranking
  if (isFuzzy && keyword) {
    const kw = keyword.toLowerCase();
    for (const result of results) {
      const idDist = distance(result.PackageIdentifier.toLowerCase(), kw);
      const nameDist = distance(result.PackageName.toLowerCase(), kw);
      result._fuzzyScore = Math.min(idDist, nameDist);
    }
    results.sort((a, b) => (a._fuzzyScore ?? Infinity) - (b._fuzzyScore ?? Infinity));
  }

  // Apply maximumResults
  const trimmed = maximumResults ? results.slice(0, maximumResults) : results;
  for (const r of trimmed) {
    delete r._fuzzyScore;
  }
  const hasMore = results.length > trimmed.length;

  return { results: trimmed, hasMore };
}
