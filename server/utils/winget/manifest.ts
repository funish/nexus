import { cacheStorage } from "../storage";
import { GITHUB_RAW_BASE, CACHE_PREFIX } from "./constants";
import { getLetterDirectoryShas, getGitHubTreePaths } from "./tree";
import type { PackageIdentifier, PackageVersion } from "./types";

/**
 * Construct the GitHub raw path for a manifest file without tree API discovery.
 * Follows the standard WinGet community manifests naming convention.
 */
export function constructManifestPath(
  packageId: PackageIdentifier,
  version: PackageVersion,
  type: "main" | "installer" | "locale",
  locale?: string,
): string {
  const parts = packageId.split(".");
  const publisher = parts[0] ?? "";
  const name = parts.slice(1).join("/");
  const letter = publisher[0]?.toLowerCase() ?? "";

  let filename: string;
  switch (type) {
    case "main":
      filename = `${packageId}.yaml`;
      break;
    case "installer":
      filename = `${packageId}.installer.yaml`;
      break;
    case "locale":
      filename = `${packageId}.locale.${locale}.yaml`;
      break;
  }

  return `manifests/${letter}/${publisher}/${name}/${version}/${filename}`;
}

export function getManifestCacheKey(manifestPath: string): string {
  return `${CACHE_PREFIX}/files/${manifestPath}`;
}

/**
 * Fetch manifest file content from GitHub raw URL with caching.
 * Manifest content is immutable per path, so no TTL needed.
 */
export async function fetchManifestContent(manifestPath: string): Promise<string> {
  const cacheKey = getManifestCacheKey(manifestPath);

  // Try cache
  const cached = await cacheStorage.getItem(cacheKey);
  if (cached && typeof cached === "string") {
    return cached;
  }

  // Cache miss — fetch from GitHub
  const rawUrl = `${GITHUB_RAW_BASE}/${manifestPath}`;
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
 * Get ALL manifest file paths for a specific version.
 * Uses cached GitHub tree paths to discover files dynamically.
 *
 * Returns full paths like:
 *   manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.yaml
 *   manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.installer.yaml
 *   manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.locale.en-US.yaml
 *   manifests/m/Microsoft/VisualStudioCode/1.95.0/Microsoft.VisualStudioCode.locale.zh-CN.yaml
 */
export async function getVersionManifests(
  packageId: PackageIdentifier,
  version: PackageVersion,
): Promise<string[]> {
  const parts = packageId.split(".");
  if (parts.length < 2) return [];

  const publisher = parts[0];
  const name = parts.slice(1).join("/");
  if (!publisher || !name) return [];

  const firstChar = publisher[0];
  if (!firstChar) return [];

  const letter = firstChar.toLowerCase();

  // Get letter directory paths from cache
  const letterShas = await getLetterDirectoryShas();
  const sha = letterShas.get(letter);
  if (!sha) return [];

  const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);
  const pathPrefix = `${publisher}/${name}/${version}/`;

  return paths
    .filter((path) => path.startsWith(pathPrefix) && path.endsWith(".yaml"))
    .map((path) => `manifests/${letter}/${path}`);
}

/**
 * Get the letter and tree paths for a specific package.
 * Convenience function used by multiple modules.
 */
export async function getPackageLetterPaths(
  packageId: PackageIdentifier,
): Promise<{ letter: string; paths: string[] } | null> {
  const parts = packageId.split(".");
  if (parts.length < 2) return null;

  const firstChar = parts[0]?.[0];
  if (!firstChar) return null;

  const letter = firstChar.toLowerCase();
  const letterShas = await getLetterDirectoryShas();
  const sha = letterShas.get(letter);
  if (!sha) return null;

  const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);
  return { letter, paths };
}
