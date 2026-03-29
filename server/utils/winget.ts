import { Database } from "bun:sqlite";

import { unzipSync } from "fflate";
import { type H3Event } from "nitro/h3";

import { cacheStorage } from "./storage";

const GITHUB_REPO = "microsoft/winget-pkgs";
const GITHUB_BRANCH = "master";

const SOURCE_MSIX_URL = "https://cdn.winget.microsoft.com/cache/source.msix";
const INDEX_DB_KEY = "registry/winget/index.db";
const UPDATE_INTERVAL = 600; // 10 minutes

/**
 * WinGet API types and utilities
 * Based on WinGet.RestSource OpenAPI specification v1.9.0
 */

/**
 * Package identifier in WinGet format
 * Example: Microsoft.VisualStudioCode
 */
export type PackageIdentifier = string;

/**
 * Package version
 * Example: 1.95.0
 */
export type PackageVersion = string;

/**
 * WinGet package metadata
 */
export interface WinGetPackage {
  PackageIdentifier: PackageIdentifier;
  Versions: PackageVersion[];
}

/**
 * Single package response
 */
export interface PackageSingleResponse {
  Data: {
    PackageIdentifier: PackageIdentifier;
  };
}

/**
 * Multiple packages response (with pagination)
 */
export interface PackageMultipleResponse {
  Data: WinGetPackage[];
  ContinuationToken?: string;
}

/**
 * Version Schema (WinGet 1.9.0)
 */
export interface VersionSchema {
  PackageVersion: PackageVersion;
  DefaultLocale: string;
  Channel?: string;
}

/**
 * Single version response (WinGet 1.9.0)
 * ResponseObjectSchema + Data: VersionSchema
 */
export interface VersionSingleResponse {
  Data: VersionSchema;
}

/**
 * Multiple versions response (WinGet 1.9.0)
 * ResponseObjectSchema + Data: VersionSchema[]
 */
export interface VersionMultipleResponse {
  Data: VersionSchema[];
  ContinuationToken?: string;
}

/**
 * Locale Schema (WinGet 1.9.0)
 */
export interface LocaleSchema {
  PackageLocale: string;
  [key: string]: any;
}

/**
 * Single locale response (WinGet 1.9.0)
 */
export interface LocaleSingleResponse {
  Data: LocaleSchema;
}

/**
 * Multiple locales response (WinGet 1.9.0)
 */
export interface LocaleMultipleResponse {
  Data: LocaleSchema[];
  ContinuationToken?: string;
}

/**
 * Installer Schema (WinGet 1.9.0)
 */
export interface InstallerSchema {
  InstallerIdentifier: string;
  [key: string]: any;
}

/**
 * Single installer response (WinGet 1.9.0)
 */
export interface InstallerSingleResponse {
  Data: InstallerSchema;
}

/**
 * Multiple installers response (WinGet 1.9.0)
 */
export interface InstallerMultipleResponse {
  Data: InstallerSchema[];
  ContinuationToken?: string;
}

/**
 * @deprecated Use VersionSchema instead
 * Kept for backward compatibility
 */
export interface WinGetVersion {
  PackageVersion: PackageVersion;
  DefaultLocale?: string;
  Locales?: string[];
  Installers?: string[];
}

/**
 * Manifest file content
 */
export interface ManifestContent {
  [key: string]: any;
}

/**
 * Parsed manifest structure for a version
 */
export interface VersionManifests {
  Version: PackageVersion;
  DefaultLocale?: string;
  Manifest?: ManifestContent;
  LocaleManifests?: Record<string, ManifestContent>;
  InstallerManifest?: ManifestContent;
}

/**
 * Error response (WinGet REST Source format: array)
 */
export interface WinGetError {
  ErrorCode: number;
  ErrorMessage: string;
}

/**
 * Create a WinGet-compatible HTTP error response
 */
export function createWinGetError(event: H3Event, status: number, message: string): WinGetError[] {
  event.res.status = status;
  return [{ ErrorCode: status, ErrorMessage: message }];
}

/**
 * Match type for search queries
 */
export type MatchType =
  | "Exact"
  | "CaseInsensitive"
  | "StartsWith"
  | "Substring"
  | "Wildcard"
  | "Fuzzy"
  | "FuzzySubstring";

/**
 * Package match field for search
 */
export type PackageMatchField =
  | "PackageIdentifier"
  | "PackageName"
  | "Moniker"
  | "Command"
  | "Tag"
  | "PackageFamilyName"
  | "ProductCode"
  | "UpgradeCode"
  | "NormalizedPackageNameAndPublisher"
  | "Market"
  | "HasInstallerType";

/**
 * Search request match
 */
export interface SearchRequestMatch {
  KeyWord?: string;
  MatchType?: MatchType;
}

/**
 * Search request package match filter (Inclusions/Filters item)
 * Spec: { PackageMatchField, RequestMatch: { KeyWord, MatchType } }
 */
export interface SearchRequestPackageMatchFilter {
  PackageMatchField: PackageMatchField;
  RequestMatch: SearchRequestMatch;
}

/**
 * Manifest search request
 */
export interface ManifestSearchRequest {
  MaximumResults?: number;
  FetchAllManifests?: boolean;
  Query?: SearchRequestMatch;
  Inclusions?: SearchRequestPackageMatchFilter[];
  Filters?: SearchRequestPackageMatchFilter[];
}

/**
 * Manifest search version response
 */
export interface ManifestSearchVersionResponse {
  PackageVersion: PackageVersion;
  Channel?: string;
  PackageFamilyNames?: string[];
  ProductCodes?: string[];
  AppsAndFeaturesEntryVersions?: string[];
  UpgradeCodes?: string[];
}

/**
 * Manifest search response
 */
export interface ManifestSearchResponse {
  PackageIdentifier: PackageIdentifier;
  PackageName: string;
  Publisher: string;
  Versions: ManifestSearchVersionResponse[];
}

/**
 * Manifest search result
 */
export interface ManifestSearchResult {
  Data: ManifestSearchResponse[];
  RequiredPackageMatchFields?: PackageMatchField[];
  UnsupportedPackageMatchFields?: PackageMatchField[];
}

// --- index.db management ---

/** Cached Database instance */
let cachedDb: Database | null = null;
/** Timestamp when cachedDb was loaded */
let cachedDbTime = 0;

/**
 * Create missing performance indexes on map tables.
 * tags_map and commands_map lack manifest indexes in the source index.db,
 * which causes NOT EXISTS subqueries to do full table scans (127k rows).
 */
export function createMissingIndexes(db: Database): void {
  db.run("CREATE INDEX IF NOT EXISTS tags_map_manifest_idx ON tags_map(manifest)");
  db.run("CREATE INDEX IF NOT EXISTS commands_map_manifest_idx ON commands_map(manifest)");
}

/**
 * Download source.msix and extract Public/index.db into cacheStorage,
 * then update the in-memory cached instance
 */
export async function refreshIndexDb(): Promise<void> {
  const response = await fetch(SOURCE_MSIX_URL);
  if (!response.ok) {
    throw new Error(`Failed to download source.msix: ${response.status}`);
  }

  const arrayBuffer = await response.arrayBuffer();
  const files = unzipSync(new Uint8Array(arrayBuffer));

  const indexDb = files["Public/index.db"];
  if (!indexDb) {
    throw new Error("index.db not found in source.msix");
  }

  await cacheStorage.setItemRaw(INDEX_DB_KEY, indexDb);
  await cacheStorage.setMeta(INDEX_DB_KEY, { mtime: new Date() });

  // Replace in-memory instance
  cachedDb?.close();
  cachedDb = Database.deserialize(indexDb);
  createMissingIndexes(cachedDb);
  cachedDbTime = Date.now();
}

/**
 * Get index.db as an in-memory SQLite Database instance.
 * Returns the cached instance if fresh, otherwise refreshes.
 */
export async function getIndexDb(event?: H3Event): Promise<Database> {
  const age = cachedDb ? (Date.now() - cachedDbTime) / 1000 : Infinity;

  if (cachedDb && age < UPDATE_INTERVAL) {
    createMissingIndexes(cachedDb);
    return cachedDb;
  }

  // Try to load from cacheStorage (e.g. across restarts)
  if (!cachedDb) {
    const data = await cacheStorage.getItemRaw(INDEX_DB_KEY);
    if (data) {
      const meta = await cacheStorage.getMeta(INDEX_DB_KEY);
      cachedDb = Database.deserialize(new Uint8Array(data as ArrayBuffer));
      createMissingIndexes(cachedDb);
      cachedDbTime = meta?.mtime ? new Date(meta.mtime).getTime() : Date.now();

      // Check if loaded data is still fresh
      if ((Date.now() - cachedDbTime) / 1000 < UPDATE_INTERVAL) {
        return cachedDb;
      }
    }
  }

  // Stale — return old instance and trigger background refresh
  if (cachedDb && event) {
    event.waitUntil(
      (async () => {
        try {
          await refreshIndexDb();
        } catch (error) {
          console.error("Failed to refresh index.db in background:", error);
        }
      })(),
    );
    return cachedDb;
  }

  // Missing or no event — download synchronously
  await refreshIndexDb();
  return cachedDb!;
}

// --- Package index ---

/**
 * Build package index from index.db
 * Map<PackageIdentifier, Set<PackageVersion>>
 */
export async function buildPackageIndex(
  event?: H3Event,
): Promise<Map<PackageIdentifier, Set<PackageVersion>>> {
  const db = await getIndexDb(event);

  const rows = db
    .query<{ id: string; version: string }, []>(
      `SELECT i.id, v.version
       FROM manifest m
       JOIN ids i ON m.id = i.rowid
       JOIN versions v ON m.version = v.rowid`,
    )
    .all();

  const index = new Map<PackageIdentifier, Set<PackageVersion>>();
  for (const row of rows) {
    if (!index.has(row.id)) {
      index.set(row.id, new Set());
    }
    index.get(row.id)!.add(row.version);
  }

  return index;
}

// --- Version manifest paths ---

/**
 * Construct deterministic GitHub manifest file paths from PackageIdentifier + Version
 */
export function getVersionManifests(
  packageId: PackageIdentifier,
  version: PackageVersion,
): string[] {
  const parts = packageId.split(".");
  if (parts.length < 2) return [];

  const publisher = parts[0];
  const name = parts.slice(1).join("/");
  if (!publisher || !name) return [];

  const letter = publisher[0]?.toLowerCase();
  if (!letter) return [];

  const basePath = `manifests/${letter}/${publisher}/${name}/${version}`;

  return [`${basePath}/${packageId}.yaml`, `${basePath}/${packageId}.installer.yaml`];
}

/**
 * Get the default locale manifest path for a specific version
 */
export function getDefaultLocaleManifestPath(
  packageId: PackageIdentifier,
  version: PackageVersion,
  locale: string = "en-US",
): string {
  const parts = packageId.split(".");
  if (parts.length < 2) return "";

  const publisher = parts[0];
  const name = parts.slice(1).join("/");
  if (!publisher || !name) return "";

  const letter = publisher[0]?.toLowerCase();
  if (!letter) return "";

  return `manifests/${letter}/${publisher}/${name}/${version}/${packageId}.locale.${locale}.yaml`;
}

// --- Search ---

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
      return null; // Handled via EXISTS subquery
    case "Tag":
      return null; // Handled via EXISTS subquery
    case "PackageFamilyName":
      return null; // Handled via EXISTS subquery
    case "ProductCode":
      return null; // Handled via EXISTS subquery
    case "UpgradeCode":
      return null; // Handled via EXISTS subquery
    case "NormalizedPackageNameAndPublisher":
      return null; // Special handling — joins two tables
    case "Market":
      return null; // Not supported
    case "HasInstallerType":
      return null; // Not supported
    default:
      return null;
  }
}

/**
 * Search packages with multi-field support
 */
export function searchPackages(
  db: Database,
  options: {
    keyword?: string;
    matchType?: MatchType;
    maximumResults?: number;
    inclusions?: SearchRequestPackageMatchFilter[];
    filters?: SearchRequestPackageMatchFilter[];
  },
): ManifestSearchResponse[] {
  const { keyword, matchType, maximumResults, inclusions, filters } = options;

  // Build WHERE clauses
  const conditions: string[] = [];
  const params: string[] = [];

  // Main keyword search across ids, names, monikers
  if (keyword) {
    const pattern = toSqlPattern(keyword, matchType || "CaseInsensitive");
    conditions.push(
      `(i.id LIKE ?1 COLLATE NOCASE OR n.name LIKE ?1 COLLATE NOCASE OR mk.moniker LIKE ?1 COLLATE NOCASE)`,
    );
    params.push(pattern);
  }

  // Inclusions (AND semantics — all must match)
  if (inclusions) {
    for (const inc of inclusions) {
      if (!inc.RequestMatch?.KeyWord || !inc.PackageMatchField) continue;

      const column = getFieldTable(inc.PackageMatchField);
      const pattern = toSqlPattern(
        inc.RequestMatch.KeyWord,
        inc.RequestMatch.MatchType || "CaseInsensitive",
      );
      const paramIdx = params.length + 1;

      if (!column) {
        conditions.push(buildSubqueryCondition(inc.PackageMatchField, paramIdx, false));
      } else {
        conditions.push(`${column} LIKE ?${paramIdx} COLLATE NOCASE`);
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
      const paramIdx = params.length + 1;

      if (!column) {
        conditions.push(buildSubqueryCondition(f.PackageMatchField, paramIdx, true));
      } else {
        conditions.push(`${column} NOT LIKE ?${paramIdx} COLLATE NOCASE`);
      }
      params.push(pattern);
    }
  }

  const whereClause = conditions.length > 0 ? `WHERE ${conditions.join(" AND ")}` : "";
  const limitClause = maximumResults ? `LIMIT ${maximumResults}` : "";

  const sql = `
    SELECT DISTINCT i.id, n.name, v.version, ch.channel, np.norm_publisher
    FROM manifest m
    JOIN ids i ON m.id = i.rowid
    JOIN names n ON m.name = n.rowid
    JOIN versions v ON m.version = v.rowid
    JOIN monikers mk ON m.moniker = mk.rowid
    JOIN channels ch ON m.channel = ch.rowid
    LEFT JOIN norm_publishers_map npm ON npm.manifest = m.rowid
    LEFT JOIN norm_publishers np ON np.rowid = npm.norm_publisher
    ${whereClause}
    ${limitClause}
  `;

  const rows = db
    .query<
      { id: string; name: string; version: string; channel: string; norm_publisher: string | null },
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
      entry = { name: row.name, publisher: row.norm_publisher || "", versions: [] };
      packageMap.set(row.id, entry);
    }
    // Prefer non-empty publisher across versions
    if (row.norm_publisher && !entry.publisher) {
      entry.publisher = row.norm_publisher;
    }
    entry.versions.push({
      PackageVersion: row.version,
      ...(row.channel && { Channel: row.channel }),
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

  return results;
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
      return "1=1"; // no-op for unsupported fields
  }
}

// --- Manifest content fetching ---

/**
 * Fetch manifest file content directly from GitHub raw URL with caching
 */
export async function fetchManifestContent(manifestPath: string): Promise<string> {
  const cacheKey = `registry/winget/${GITHUB_REPO}/files/${manifestPath}`;

  // Try to get from cache
  const cached = await cacheStorage.getItem(cacheKey);
  if (cached && typeof cached === "string") {
    return cached;
  }

  // Cache miss - fetch from GitHub
  const rawUrl = `https://raw.githubusercontent.com/${GITHUB_REPO}/${GITHUB_BRANCH}/${manifestPath}`;
  const response = await fetch(rawUrl);

  if (!response.ok) {
    throw new Error(`Failed to fetch manifest: ${response.statusText}`);
  }

  const content = await response.text();

  // Store in cache
  await cacheStorage.setItem(cacheKey, content);

  return content;
}

/**
 * Compare two version strings, falling back to numeric comparison for non-semver versions.
 */
export function compareVersion(a: string, b: string): number {
  try {
    return Bun.semver.order(a, b);
  } catch {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    const len = Math.max(pa.length, pb.length);
    for (let i = 0; i < len; i++) {
      const x = pa[i] || 0;
      const y = pb[i] || 0;
      if (x !== y) return x - y;
    }
    return 0;
  }
}
