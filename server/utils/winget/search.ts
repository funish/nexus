import { Database } from "bun:sqlite";

import { FuzzySearch, type ISearchKey } from "@nlptools/distance";

import { cacheStorage, memoryStorage } from "../storage";
import { WINGET_SEARCH_INDEX_KEY } from "./constants";
import { decodeContinuationToken } from "./token";
import type {
  WinGetMatchType,
  WinGetManifestSearchResponse,
  WinGetManifestSearchVersionResponse,
  WinGetPackageMatchField,
  WinGetSearchEntry,
  WinGetSearchRequestPackageMatchFilter,
} from "./types";
import { compareVersion } from "./version";

// ── Search index build/cache ──────────────────────────

const DELIM = "\x1E";

/**
 * Build a unified search index from index.db.
 * Fetches all fields in one query so search responses need no second DB hit.
 */
export function buildSearchIndex(db: Database): WinGetSearchEntry[] {
  const rows = db
    .query<
      {
        id: string;
        name: string;
        norm_publisher: string | null;
        monikers: string | null;
        tags: string | null;
        commands: string | null;
        version: string;
        channel: string | null;
        pfns: string | null;
        productcodes: string | null;
        upgradecodes: string | null;
      },
      string[]
    >(`
    SELECT DISTINCT i.id, n.name, np.norm_publisher,
      (SELECT GROUP_CONCAT(moniker, '${DELIM}') FROM (SELECT DISTINCT mk.moniker FROM manifest m2 JOIN monikers mk ON m2.moniker = mk.rowid WHERE m2.id = m.id AND mk.moniker != '') as sub) as monikers,
      (SELECT GROUP_CONCAT(tag, '${DELIM}') FROM (SELECT DISTINCT t.tag FROM tags_map tm2 JOIN tags t ON t.rowid = tm2.tag JOIN manifest m2 ON m2.rowid = tm2.manifest WHERE m2.id = m.id) as sub) as tags,
      (SELECT GROUP_CONCAT(command, '${DELIM}') FROM (SELECT DISTINCT c.command FROM commands_map cm2 JOIN commands c ON c.rowid = cm2.command JOIN manifest m2 ON m2.rowid = cm2.manifest WHERE m2.id = m.id) as sub) as commands,
      v.version, ch.channel,
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
  `)
    .all();

  const split = (s: string | null): string[] =>
    s ? [...new Set(s.split(DELIM).filter(Boolean))] : [];

  const entryMap = new Map<string, WinGetSearchEntry>();

  for (const r of rows) {
    let entry = entryMap.get(r.id);
    if (!entry) {
      entry = {
        id: r.id,
        name: r.name,
        publisher: r.norm_publisher || r.id.split(".")[0] || "",
        monikers: split(r.monikers),
        tags: split(r.tags),
        commands: split(r.commands),
        versions: [],
        packageFamilyNames: [],
        productCodes: [],
        upgradeCodes: [],
      };
      entryMap.set(r.id, entry);
    }

    // Deduplicate: same version+channel may appear due to LEFT JOIN fan-out
    const alreadyExists = entry.versions.some(
      (v) => v.PackageVersion === r.version && (v.Channel || null) === (r.channel || null),
    );
    if (!alreadyExists) {
      const versionEntry: WinGetManifestSearchVersionResponse = {
        PackageVersion: r.version,
      };
      if (r.channel) versionEntry.Channel = r.channel;

      entry.versions.push(versionEntry);
    }

    // Merge per-version extra fields into entry-level sets
    const pfns = split(r.pfns);
    for (const p of pfns) {
      if (!entry.packageFamilyNames.includes(p)) entry.packageFamilyNames.push(p);
    }
    const pcs = split(r.productcodes);
    for (const p of pcs) {
      if (!entry.productCodes.includes(p)) entry.productCodes.push(p);
    }
    const ucs = split(r.upgradecodes);
    for (const u of ucs) {
      if (!entry.upgradeCodes.includes(u)) entry.upgradeCodes.push(u);
    }
  }

  // Sort versions descending within each entry
  for (const entry of entryMap.values()) {
    entry.versions.sort((a, b) => compareVersion(b.PackageVersion, a.PackageVersion));
  }

  return [...entryMap.values()];
}

/**
 * Persist search index to cacheStorage and load into memoryStorage.
 */
export async function persistSearchIndex(index: WinGetSearchEntry[]): Promise<void> {
  try {
    await cacheStorage.setItem(WINGET_SEARCH_INDEX_KEY, index);
    await cacheStorage.setMeta(WINGET_SEARCH_INDEX_KEY, { mtime: new Date() });
  } catch (e) {
    console.error("[search] persistSearchIndex failed:", e);
  }
  await memoryStorage.setItem(WINGET_SEARCH_INDEX_KEY, index);
}

/**
 * Retrieve the search index.
 * Loads from memoryStorage first, falls back to cacheStorage, then returns null.
 */
export async function getSearchIndex(): Promise<WinGetSearchEntry[] | null> {
  // Fast path: in-memory cache
  const memCached = await memoryStorage.getItem(WINGET_SEARCH_INDEX_KEY);
  if (memCached) {
    return memCached as WinGetSearchEntry[];
  }

  // Slow path: persistent cache (unstorage handles deserialization)
  const cached = await cacheStorage.getItem(WINGET_SEARCH_INDEX_KEY);
  if (cached) {
    const index = cached as WinGetSearchEntry[];
    await memoryStorage.setItem(WINGET_SEARCH_INDEX_KEY, index);
    return index;
  }

  return null;
}

// ── Search configuration ──────────────────────────────

/** Search keys with weights matching fuse.js config */
const SEARCH_KEYS: ISearchKey[] = [
  { name: "id", weight: 2 },
  { name: "name", weight: 2 },
  { name: "publisher", weight: 1 },
  { name: "monikers", weight: 1.5 },
  { name: "tags", weight: 0.5 },
  { name: "commands", weight: 1.5 },
  { name: "packageFamilyNames", weight: 1 },
  { name: "productCodes", weight: 1 },
  { name: "upgradeCodes", weight: 1 },
];

/** Map WinGet PackageMatchField to WinGetSearchEntry key */
const FIELD_TO_KEY: Partial<Record<WinGetPackageMatchField, string>> = {
  PackageIdentifier: "id",
  PackageName: "name",
  Publisher: "publisher",
  Moniker: "monikers",
  Command: "commands",
  Tag: "tags",
  PackageFamilyName: "packageFamilyNames",
  ProductCode: "productCodes",
  UpgradeCode: "upgradeCodes",
};

// ── Match helpers ─────────────────────────────────────

/** Match a single string value against a keyword with the given match type */
export function matchString(value: string, keyword: string, matchType?: WinGetMatchType): boolean {
  const lv = value.toLowerCase();
  const lk = keyword.toLowerCase();

  switch (matchType) {
    case "Exact":
      return lv === lk;
    case "CaseInsensitive":
      return lv.includes(lk);
    case "StartsWith":
      return lv.startsWith(lk);
    case "Substring":
      return lv.includes(lk);
    case "Wildcard":
      // Simple glob: treat as case-insensitive substring for now
      return lv.includes(lk);
    case "Fuzzy":
    case "FuzzySubstring":
      // Fuzzy matching handled by FuzzySearch; this is used for filters/inclusions
      // Fall through to case-insensitive substring as a reasonable default
      return lv.includes(lk);
    default:
      return lv.includes(lk);
  }
}

/**
 * Check if an entry matches a single filter/inclusion condition.
 * For array fields (monikers, tags, commands, etc.), checks if any element matches.
 */
export function matchesField(
  entry: WinGetSearchEntry,
  fieldName: string,
  keyword: string,
  matchType?: WinGetMatchType,
): boolean {
  const value = (entry as unknown as Record<string, unknown>)[fieldName];
  if (typeof value === "string") {
    return matchString(value, keyword, matchType);
  }
  if (Array.isArray(value)) {
    return value.some((v) => typeof v === "string" && matchString(v, keyword, matchType));
  }
  return false;
}

/**
 * Apply inclusions (AND): entry must match ALL inclusions.
 * NormalizedPackageNameAndPublisher maps to name OR publisher.
 */
export function matchesInclusions(
  entry: WinGetSearchEntry,
  inclusions: WinGetSearchRequestPackageMatchFilter[],
): boolean {
  return inclusions.every((inc) => {
    if (!inc.RequestMatch?.KeyWord) return true;
    const kw = inc.RequestMatch.KeyWord;
    const mt = inc.RequestMatch.MatchType;

    if (inc.PackageMatchField === "NormalizedPackageNameAndPublisher") {
      return matchString(entry.name, kw, mt) || matchString(entry.publisher, kw, mt);
    }

    const key = FIELD_TO_KEY[inc.PackageMatchField];
    if (!key) return true;
    return matchesField(entry, key, kw, mt);
  });
}

/**
 * Apply filters (AND): entry must NOT match ANY filter.
 */
export function matchesFilters(
  entry: WinGetSearchEntry,
  filters: WinGetSearchRequestPackageMatchFilter[],
): boolean {
  return !filters.some((f) => {
    if (!f.RequestMatch?.KeyWord) return false;
    const kw = f.RequestMatch.KeyWord;
    const mt = f.RequestMatch.MatchType;

    if (f.PackageMatchField === "NormalizedPackageNameAndPublisher") {
      return matchString(entry.name, kw, mt) || matchString(entry.publisher, kw, mt);
    }

    const key = FIELD_TO_KEY[f.PackageMatchField];
    if (!key) return false;
    return matchesField(entry, key, kw, mt);
  });
}

// ── FuzzySearch configuration per match type ──────────

/**
 * Check if any searchable field of an entry matches the keyword, and return
 * a relevance score for sorting. Higher score = more relevant.
 */
export function scoreEntryKeyword(
  entry: WinGetSearchEntry,
  keyword: string,
  matchType?: WinGetMatchType,
): number {
  const kw = keyword.toLowerCase();
  const fields = [
    entry.id,
    entry.name,
    entry.publisher,
    ...entry.monikers,
    ...entry.tags,
    ...entry.commands,
    ...entry.packageFamilyNames,
    ...entry.productCodes,
    ...entry.upgradeCodes,
  ];

  let best = 0;

  for (let i = 0; i < fields.length; i++) {
    const f = fields[i]!.toLowerCase();
    let matched = false;
    let score = 0;

    switch (matchType) {
      case "Exact":
        matched = f === kw;
        score = matched ? 1000 - i : 0;
        break;
      case "CaseInsensitive":
        // WinGet CaseInsensitive = case-insensitive substring match (not exact)
        matched = f.includes(kw);
        score = matched ? 1000 - i + (kw.length / f.length) * 100 : 0;
        break;
      case "StartsWith":
        matched = f.startsWith(kw);
        score = matched ? 1000 - i + (kw.length / f.length) * 100 : 0;
        break;
      case "Substring":
      case "Wildcard":
      case "FuzzySubstring":
        matched = f.includes(kw);
        score = matched ? 1000 - i + (kw.length / f.length) * 100 : 0;
        break;
      default:
        matched = f.includes(kw);
        score = matched ? 1000 - i + (kw.length / f.length) * 100 : 0;
    }

    if (score > best) best = score;
    if (matched && (matchType === "Exact" || matchType === "CaseInsensitive")) return best;
  }

  return best;
}

// ── Main search function ──────────────────────────────

/**
 * Search packages using @nlptools/distance FuzzySearch.
 * All data comes from the in-memory search index — no DB query needed.
 */
export function searchPackages(options: {
  keyword?: string;
  matchType?: WinGetMatchType;
  maximumResults?: number;
  continuationToken?: string;
  inclusions?: WinGetSearchRequestPackageMatchFilter[];
  filters?: WinGetSearchRequestPackageMatchFilter[];
  searchIndex: WinGetSearchEntry[];
}): { results: WinGetManifestSearchResponse[]; hasMore: boolean; offset: number } {
  const {
    keyword,
    matchType,
    maximumResults,
    continuationToken,
    inclusions,
    filters,
    searchIndex,
  } = options;

  const offset = decodeContinuationToken(continuationToken);

  let matchedEntries: WinGetSearchEntry[];

  if (!keyword && !inclusions?.length && !filters?.length) {
    matchedEntries = searchIndex;
  } else {
    let candidates: WinGetSearchEntry[];

    if (keyword) {
      const isFuzzy = matchType === "Fuzzy" || matchType === "FuzzySubstring";

      if (isFuzzy) {
        // Use FuzzySearch with levenshtein for fuzzy matching
        const engine = new FuzzySearch(searchIndex, {
          keys: SEARCH_KEYS,
          algorithm: "levenshtein",
          threshold: matchType === "Fuzzy" ? 0.15 : 0.1,
          caseSensitive: false,
        });
        candidates = engine.search(keyword).map((r) => r.item);
      } else {
        // Linear scan for exact/prefix/substring — fast and correct
        candidates = searchIndex
          .map((e) => ({ entry: e, score: scoreEntryKeyword(e, keyword, matchType) }))
          .filter((e) => e.score > 0)
          .sort((a, b) => b.score - a.score)
          .map((e) => e.entry);
      }
    } else {
      candidates = searchIndex;
    }

    // Apply inclusions (AND)
    if (inclusions?.length) {
      candidates = candidates.filter((e) => matchesInclusions(e, inclusions));
    }

    // Apply filters (NOT)
    if (filters?.length) {
      candidates = candidates.filter((e) => matchesFilters(e, filters));
    }

    matchedEntries = candidates;
  }

  const results: WinGetManifestSearchResponse[] = matchedEntries.map((entry) => ({
    PackageIdentifier: entry.id,
    PackageName: entry.name,
    Publisher: entry.publisher,
    Versions: entry.versions,
  }));

  const trimmed = maximumResults
    ? results.slice(offset, offset + maximumResults)
    : results.slice(offset);
  const hasMore = results.length > offset + trimmed.length;

  return { results: trimmed, hasMore, offset };
}
