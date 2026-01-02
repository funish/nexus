import type { H3Event } from "nitro/h3";
import { useStorage } from "nitro/storage";

const GITHUB_REPO = "microsoft/winget-pkgs";
const GITHUB_BRANCH = "master";
const GITHUB_API_BASE = "https://api.github.com";

/**
 * Get GitHub authentication headers if token is available
 */
function getGitHubHeaders(): HeadersInit {
  const token = process.env.GITHUB_TOKEN;
  const headers: HeadersInit = {
    "User-Agent": "Funish Nexus",
  };

  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  return headers;
}

/**
 * WinGet Registry types and utilities
 * Based on WinGet.RestSource OpenAPI specification v1.9.0
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
  Publisher?: string;
  PackageName?: string;
  ShortDescription?: string;
  Description?: string;
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
  InstallerIdentifier?: string;
  InstallerType?: string;
  InstallerUrl?: string;
  Architecture?: string;
  Scope?: string;
  Language?: string;
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
 * Fetch GitHub tree data by SHA or branch
 * @param treeSha - The SHA of the tree or branch name
 * @param recursive - Whether to fetch recursively (default: false)
 */
export async function getGitHubTree(
  treeSha: string = GITHUB_BRANCH,
  recursive: boolean = false,
): Promise<GitHubTreeResponse> {
  // Fetch from GitHub API
  const url = `${GITHUB_API_BASE}/repos/${GITHUB_REPO}/git/trees/${treeSha}${recursive ? "?recursive=1" : ""}`;
  const response = await fetch(url, {
    headers: getGitHubHeaders(),
  });

  if (!response.ok) {
    throw new Error(`Failed to fetch GitHub tree: ${response.statusText}`);
  }

  return (await response.json()) as GitHubTreeResponse;
}

/**
 * Fetch and cache GitHub tree paths (optimized storage)
 * @param treeSha - The SHA of the tree
 * @param cacheSuffix - Cache suffix for this tree
 * @returns Array of file paths
 */
export async function getGitHubTreePaths(treeSha: string, cacheSuffix: string): Promise<string[]> {
  const storage = useStorage("cache");
  // Replace slashes with dashes to avoid filesystem nesting issues
  const normalizedSuffix = cacheSuffix.replace(/\//g, "-");
  const cacheKey = `registry/winget/${GITHUB_REPO}/${normalizedSuffix}`;
  const UPDATE_INTERVAL = 600; // 10 minutes

  // Check cache metadata
  const meta = await storage.getMeta(cacheKey);
  const now = new Date();

  // If cache exists and is within update interval, return it
  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < UPDATE_INTERVAL) {
      const cached = await storage.getItem(cacheKey);
      if (cached && Array.isArray(cached)) {
        return cached as string[];
      }
    }
  }

  // Fetch tree data
  const treeData = await getGitHubTree(treeSha, true);

  // Extract only paths from tree items
  const paths = treeData.tree.map((item) => item.path);

  // Cache the paths array
  await storage.setItem(cacheKey, paths);
  await storage.setMeta(cacheKey, { mtime: new Date() });

  return paths;
}

/**
 * Filter tree to only include manifest files
 */
export function filterManifestFiles(tree: GitHubTreeItem[]): GitHubTreeItem[] {
  return tree.filter((item) => item.path.startsWith("manifests/"));
}

/**
 * Get the SHA of the manifests directory
 */
export async function getManifestsSha(): Promise<string> {
  const storage = useStorage("cache");
  const manifestsShaKey = `registry/winget/${GITHUB_REPO}/manifests-sha`;
  const UPDATE_INTERVAL = 600; // 10 minutes

  // Check cache metadata
  const meta = await storage.getMeta(manifestsShaKey);
  const now = new Date();

  // If cache exists and is within update interval, return it
  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < UPDATE_INTERVAL) {
      const cached = await storage.getItem(manifestsShaKey);
      if (cached && typeof cached === "string") {
        return cached;
      }
    }
  }

  // Fetch root tree
  const rootTree = await getGitHubTree(GITHUB_BRANCH, false);
  const manifestsItem = rootTree.tree.find(
    (item) => item.path === "manifests" && item.type === "tree",
  );

  if (!manifestsItem) {
    throw new Error("manifests directory not found in repository");
  }

  // Cache the SHA
  await storage.setItem(manifestsShaKey, manifestsItem.sha);
  await storage.setMeta(manifestsShaKey, { mtime: new Date() });

  return manifestsItem.sha;
}

/**
 * Get all letter directory SHAs from manifests
 */
export async function getLetterDirectoryShas(): Promise<Map<string, string>> {
  const manifestsSha = await getManifestsSha();
  const manifestsTree = await getGitHubTree(manifestsSha, false);

  const letterShas = new Map<string, string>();

  for (const item of manifestsTree.tree) {
    // Match single letter directories (a-z, 0-9)
    if (item.type === "tree" && item.path.length === 1 && /[a-z0-9]/.test(item.path)) {
      letterShas.set(item.path, item.sha);
    }
  }

  if (letterShas.size === 0) {
    throw new Error("No letter directories found in manifests");
  }

  return letterShas;
}

/**
 * Parse package identifier from manifest path
 * manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.yaml
 * → Microsoft.VisualStudioCode
 *
 * Note: When fetching from letter directory, path is relative:
 * Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.yaml
 * → Microsoft.VisualStudioCode
 */
export function parsePackageIdentifier(path: string): PackageIdentifier | null {
  // Try full path first: manifests/a/publisher/name/...
  let match = path.match(/^manifests\/[a-z0-9]\/([^/]+)\/([^/]+)\//);
  if (match) {
    const [, publisher, name] = match;
    return `${publisher}.${name}`;
  }

  // Try relative path: publisher/name/...
  match = path.match(/^([^/]+)\/([^/]+)\//);
  if (match) {
    const [, publisher, name] = match;
    return `${publisher}.${name}`;
  }

  return null;
}

/**
 * Parse version from manifest path
 * manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.yaml
 * → 1.95.0
 *
 * Note: When fetching from letter directory, path is relative:
 * Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.yaml
 * → 1.95.0
 */
export function parseVersion(path: string): PackageVersion | null {
  // Try full path first: manifests/a/publisher/name/version/...
  let match = path.match(/^manifests\/[a-z0-9]\/[^/]+\/[^/]+\/([^/]+)\//);
  if (match && match[1]) {
    return match[1];
  }

  // Try relative path: publisher/name/version/...
  match = path.match(/^[^/]+\/[^/]+\/([^/]+)\//);
  if (match && match[1]) {
    return match[1];
  }

  return null;
}

/**
 * Build package index from tree data
 * Map<PackageIdentifier, Set<Version>>
 */
export async function buildPackageIndex(
  event?: H3Event,
): Promise<Map<PackageIdentifier, Set<PackageVersion>>> {
  const storage = useStorage("cache");
  const cacheKey = `registry/winget/${GITHUB_REPO}/index`;
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
        // Convert cached object back to Map with Set values
        const cachedData = cached as Record<string, string[]>;
        const index = new Map<PackageIdentifier, Set<PackageVersion>>();
        for (const [pkgId, versions] of Object.entries(cachedData)) {
          index.set(pkgId, new Set(versions));
        }
        return index;
      }
    }
  }

  // If cache is stale but exists, return stale cache and trigger background update
  if (meta?.mtime && event) {
    const cached = await storage.getItem(cacheKey);
    if (cached) {
      const cachedData = cached as Record<string, string[]>;
      const staleIndex = new Map<PackageIdentifier, Set<PackageVersion>>();
      for (const [pkgId, versions] of Object.entries(cachedData)) {
        staleIndex.set(pkgId, new Set(versions));
      }

      // Trigger background update using event.waitUntil
      event.waitUntil(
        (async () => {
          try {
            await rebuildPackageIndex();
          } catch (error) {
            console.error("Failed to rebuild package index in background:", error);
          }
        })(),
      );

      return staleIndex;
    }
  }

  // No cache available, build synchronously
  return await rebuildPackageIndex();
}

/**
 * Rebuild the package index from GitHub
 */
async function rebuildPackageIndex(): Promise<Map<PackageIdentifier, Set<PackageVersion>>> {
  const storage = useStorage("cache");
  const cacheKey = `registry/winget/${GITHUB_REPO}/index`;

  // Get all letter directory SHAs
  const letterShas = await getLetterDirectoryShas();

  // Fetch all letter directory paths in parallel
  const letterPromises = Array.from(letterShas.entries()).map(
    async ([letter, sha]): Promise<{
      letter: string;
      paths?: string[];
      success: boolean;
    }> => {
      try {
        const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);
        return { letter, paths, success: true };
      } catch (error) {
        console.error(`Failed to fetch tree for letter ${letter}:`, error);
        return { letter, success: false };
      }
    },
  );

  const letterResults = await Promise.allSettled(letterPromises);

  // Build index from all letter paths
  const index = new Map<PackageIdentifier, Set<PackageVersion>>();

  for (const result of letterResults) {
    if (result.status === "rejected" || !result.value.success) {
      continue;
    }

    const { paths } = result.value;

    if (!paths) {
      continue;
    }

    for (const path of paths) {
      // Only process YAML files
      if (!path.endsWith(".yaml")) continue;

      const pkgId = parsePackageIdentifier(path);
      const version = parseVersion(path);

      if (pkgId && version) {
        if (!index.has(pkgId)) {
          index.set(pkgId, new Set());
        }
        index.get(pkgId)!.add(version);
      }
    }
  }

  // Convert to plain object for caching
  const cacheData: Record<string, string[]> = {};
  for (const [pkgId, versions] of index.entries()) {
    cacheData[pkgId] = Array.from(versions);
  }

  // Cache the index and set metadata
  await storage.setItem(cacheKey, cacheData);
  await storage.setMeta(cacheKey, { mtime: new Date() });

  return index;
}

/**
 * Get all manifest file paths for a specific version
 * @returns Array of manifest file paths (full repository paths)
 */
export async function getVersionManifests(
  packageId: PackageIdentifier,
  version: PackageVersion,
): Promise<string[]> {
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

  const letter = firstChar.toLowerCase();

  // Get all letter directory SHAs
  const letterShas = await getLetterDirectoryShas();
  const sha = letterShas.get(letter);

  if (!sha) {
    return [];
  }

  // Reuse cached paths (consistent with buildPackageIndex)
  const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);
  const pathPrefix = `${publisher}/${name}/${version}/`;

  return paths
    .filter((path) => path.startsWith(pathPrefix) && path.endsWith(".yaml"))
    .map((path) => `manifests/${letter}/${path}`);
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
