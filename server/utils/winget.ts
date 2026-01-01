import { useStorage } from "nitro/storage";

const GITHUB_REPO = "microsoft/winget-pkgs";
const GITHUB_BRANCH = "master";
const GITHUB_API_BASE = "https://api.github.com";

/**
 * WinGet Registry types and utilities
 * Based on WinGet.RestSource OpenAPI specification v1.1.0
 */

/**
 * GitHub Tree API response
 */
export interface GitHubTreeItem {
  path: string;
  mode: string;
  type: "tree" | "blob";
  sha: string;
  size?: number;
  url: string;
}

export interface GitHubTreeResponse {
  sha: string;
  url: string;
  tree: GitHubTreeItem[];
  truncated: boolean;
}

/**
 * Package identifier in WinGet format
 * Example: ByteDance.Doubao
 */
export type PackageIdentifier = string;

/**
 * Package version
 * Example: 1.46.7
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
  PackageIdentifier: PackageIdentifier;
  Versions: PackageVersion[];
}

/**
 * Multiple packages response (with pagination)
 */
export interface PackageMultipleResponse {
  Data: WinGetPackage[];
  ContinuationToken?: string;
}

/**
 * Version metadata
 */
export interface WinGetVersion {
  PackageVersion: PackageVersion;
  DefaultLocale?: string;
  Locales?: string[];
  Installers?: string[];
}

/**
 * Single version response with manifest content
 */
export interface VersionSingleResponse extends WinGetVersion {
  Manifest?: ManifestContent;
  LocaleManifests?: Record<string, ManifestContent>;
  InstallerManifest?: ManifestContent;
}

/**
 * Multiple versions response (with pagination)
 */
export interface VersionMultipleResponse {
  Data: WinGetVersion[];
  ContinuationToken?: string;
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
 * Error response
 */
export interface WinGetError {
  error: string;
  message?: string;
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
  | "NormalizedPackageNameAndPublisher"
  | "Market";

/**
 * Search request match
 */
export interface SearchRequestMatch {
  KeyWord?: string;
  MatchType?: MatchType;
}

/**
 * Manifest search request (adapted for GET query parameters)
 */
export interface ManifestSearchRequest {
  MaximumResults?: number;
  FetchAllManifests?: boolean;
  Query?: SearchRequestMatch;
}

/**
 * Manifest search version response
 */
export interface ManifestSearchVersionResponse {
  PackageVersion: PackageVersion;
  Channel?: string;
}

/**
 * Manifest search response
 */
export interface ManifestSearchResponse {
  PackageIdentifier: PackageIdentifier;
  PackageName?: string;
  Publisher?: string;
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

/**
 * Fetch and cache GitHub tree data
 * Updates every 10 minutes (600 seconds)
 */
export async function getGitHubTree(): Promise<GitHubTreeResponse> {
  const storage = useStorage("cache");
  const cacheKey = `registry/winget/${GITHUB_REPO}/${GITHUB_BRANCH}`;
  const UPDATE_INTERVAL = 600; // 10 minutes

  // Check cache metadata
  const meta = await storage.getMeta(cacheKey);
  const now = new Date();

  // If cache exists and is within update interval, return it
  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < UPDATE_INTERVAL) {
      const cached = await storage.getItem(cacheKey);
      if (cached) {
        return cached as GitHubTreeResponse;
      }
    }
  }

  // Fetch from GitHub API
  const url = `${GITHUB_API_BASE}/repos/${GITHUB_REPO}/git/trees/${GITHUB_BRANCH}?recursive=1`;
  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`Failed to fetch GitHub tree: ${response.statusText}`);
  }

  const data = (await response.json()) as GitHubTreeResponse;

  // Cache the response and set metadata with current time
  await storage.setItem(cacheKey, data);
  await storage.setMeta(cacheKey, { mtime: new Date() });

  return data;
}

/**
 * Filter tree to only include manifest files
 */
export function filterManifestFiles(tree: GitHubTreeItem[]): GitHubTreeItem[] {
  return tree.filter((item) => item.path.startsWith("manifests/"));
}

/**
 * Parse package identifier from manifest path
 * manifests/b/ByteDance/Doubao/1.46.7/ByteDance.Doubao.yaml
 * → ByteDance.Doubao
 */
export function parsePackageIdentifier(path: string): PackageIdentifier | null {
  const match = path.match(/^manifests\/[a-z]\/([^/]+)\/([^/]+)\//);
  if (!match) return null;

  const [, publisher, name] = match;
  return `${publisher}.${name}`;
}

/**
 * Parse version from manifest path
 * manifests/b/ByteDance/Doubao/1.46.7/ByteDance.Doubao.yaml
 * → 1.46.7
 */
export function parseVersion(path: string): PackageVersion | null {
  const match = path.match(/^manifests\/[a-z]\/[^/]+\/[^/]+\/([^/]+)\//);
  if (!match || !match[1]) return null;

  return match[1];
}

/**
 * Build package index from tree data
 * Map<PackageIdentifier, Set<Version>>
 */
export async function buildPackageIndex(): Promise<Map<PackageIdentifier, Set<PackageVersion>>> {
  const tree = await getGitHubTree();
  const manifestFiles = filterManifestFiles(tree.tree);

  const index = new Map<PackageIdentifier, Set<PackageVersion>>();

  for (const file of manifestFiles) {
    if (file.type === "blob" && file.path.endsWith(".yaml")) {
      const pkgId = parsePackageIdentifier(file.path);
      const version = parseVersion(file.path);

      if (pkgId && version) {
        if (!index.has(pkgId)) {
          index.set(pkgId, new Set());
        }
        index.get(pkgId)!.add(version);
      }
    }
  }

  return index;
}

/**
 * Get all manifest files for a specific version
 */
export async function getVersionManifests(
  packageId: PackageIdentifier,
  version: PackageVersion,
): Promise<GitHubTreeItem[]> {
  const tree = await getGitHubTree();
  const manifestFiles = filterManifestFiles(tree.tree);

  const parts = packageId.split(".");
  if (parts.length < 2) {
    return [];
  }

  const [publisher, name] = parts;
  if (!publisher || !name) {
    return [];
  }

  const firstChar = publisher[0];
  if (!firstChar) {
    return [];
  }

  const pathPrefix = `manifests/${firstChar.toLowerCase()}/${publisher}/${name}/${version}/`;

  return manifestFiles.filter((item) => item.path.startsWith(pathPrefix) && item.type === "blob");
}

/**
 * Fetch manifest file content directly from GitHub raw URL with caching
 */
export async function fetchManifestContent(manifestPath: string): Promise<string> {
  const storage = useStorage("cache");
  const cacheKey = `registry/winget/${GITHUB_REPO}/files/${manifestPath}`;

  // Try to get from cache
  const cached = await storage.getItem(cacheKey);
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
  await storage.setItem(cacheKey, content);

  return content;
}
