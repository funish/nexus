import { Database } from "bun:sqlite";

import Fuse, { type Expression, type IFuseOptions } from "fuse.js";

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

// ── Fuse.js configuration ─────────────────────────────

const FUSE_KEYS = [
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

const FUSE_OPTIONS: IFuseOptions<WinGetSearchEntry> = {
  keys: FUSE_KEYS,
  threshold: 0.4,
  includeScore: true,
  shouldSort: true,
  useExtendedSearch: true,
};

// ── Fuse instance creation ────────────────────────────

/** Map WinGet PackageMatchField to fuse.js key name */
const FIELD_TO_KEY: Partial<Record<WinGetPackageMatchField, string>> = {
  PackageIdentifier: "id",
  PackageName: "name",
  Moniker: "monikers",
  Command: "commands",
  Tag: "tags",
  PackageFamilyName: "packageFamilyNames",
  ProductCode: "productCodes",
  UpgradeCode: "upgradeCodes",
};

/**
 * Create a fuse.js instance configured for the given match type.
 */
export function createFuse(
  index: WinGetSearchEntry[],
  matchType?: WinGetMatchType,
): Fuse<WinGetSearchEntry> {
  const opts: IFuseOptions<WinGetSearchEntry> = { ...FUSE_OPTIONS, keys: FUSE_KEYS };

  if (matchType === "Exact") {
    opts.threshold = 0;
  } else if (
    matchType === "CaseInsensitive" ||
    matchType === "Substring" ||
    matchType === "StartsWith"
  ) {
    opts.threshold = 0;
    opts.ignoreLocation = true;
  } else if (matchType === "Fuzzy") {
    opts.threshold = 0.4;
  } else if (matchType === "FuzzySubstring") {
    opts.threshold = 0.4;
    opts.ignoreLocation = true;
  }

  return new Fuse(index, opts);
}

/** Map WinGet MatchType to fuse.js extended search pattern modifier */
export function toExtendedPattern(keyword: string, matchType?: WinGetMatchType): string {
  switch (matchType) {
    case "Exact":
      return `'${keyword}`;
    case "StartsWith":
      return `^${keyword}`;
    case "Wildcard":
      return keyword;
    default:
      return keyword;
  }
}

/**
 * Build a combined fuse.js extended search query.
 * Uses $and for keyword + inclusions, and ! prefix for filters (NOT).
 * NormalizedPackageNameAndPublisher maps to $or across name and publisher.
 */
export function buildSearchQuery(
  keyword?: string,
  matchType?: WinGetMatchType,
  inclusions?: WinGetSearchRequestPackageMatchFilter[],
  filters?: WinGetSearchRequestPackageMatchFilter[],
): string | Expression {
  const conditions: (string | Expression)[] = [];

  if (keyword) {
    conditions.push(toExtendedPattern(keyword, matchType));
  }

  if (inclusions) {
    for (const inc of inclusions) {
      if (!inc.RequestMatch?.KeyWord || !inc.PackageMatchField) continue;

      if (inc.PackageMatchField === "NormalizedPackageNameAndPublisher") {
        const pattern = toExtendedPattern(inc.RequestMatch.KeyWord, inc.RequestMatch.MatchType);
        conditions.push({ $or: [{ name: pattern }, { publisher: pattern }] });
        continue;
      }

      const key = FIELD_TO_KEY[inc.PackageMatchField];
      if (!key) continue;
      conditions.push({
        [key]: toExtendedPattern(inc.RequestMatch.KeyWord, inc.RequestMatch.MatchType),
      });
    }
  }

  if (filters) {
    for (const f of filters) {
      if (!f.RequestMatch?.KeyWord || !f.PackageMatchField) continue;

      if (f.PackageMatchField === "NormalizedPackageNameAndPublisher") {
        const pattern = toExtendedPattern(f.RequestMatch.KeyWord, f.RequestMatch.MatchType);
        conditions.push({
          $or: [
            { name: `!${pattern.replace(/^!/, "")}` },
            { publisher: `!${pattern.replace(/^!/, "")}` },
          ],
        });
        continue;
      }

      const key = FIELD_TO_KEY[f.PackageMatchField];
      if (!key) continue;
      const raw = toExtendedPattern(f.RequestMatch.KeyWord, f.RequestMatch.MatchType);
      conditions.push({ [key]: `!${raw.replace(/^!/, "")}` });
    }
  }

  if (conditions.length === 0) return "";
  if (conditions.length === 1) return conditions[0]!;
  return { $and: conditions as Expression[] };
}

// ── Main search function ──────────────────────────────

/**
 * Search packages using fuse.js extended search.
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
    const fuse = createFuse(searchIndex, matchType);
    const query = buildSearchQuery(keyword, matchType, inclusions, filters);
    matchedEntries = fuse.search(query).map((r) => r.item);
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
