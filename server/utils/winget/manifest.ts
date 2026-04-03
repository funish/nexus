import { parseYAML } from "confbox";

import { cacheStorage } from "../storage";
import { WINGET_GITHUB_RAW_BASE, WINGET_CACHE_PREFIX } from "./constants";
import { getLetterDirectoryShas, getGitHubTreePaths } from "./tree";
import type { WinGetPackageIdentifier, WinGetPackageVersion, WinGetVersionManifest } from "./types";

/**
 * Construct the GitHub raw path for a manifest file without tree API discovery.
 * Follows the standard WinGet community manifests naming convention.
 */
export function constructManifestPath(
  packageId: WinGetPackageIdentifier,
  version: WinGetPackageVersion,
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

/**
 * Fetch manifest file content from GitHub raw URL with caching.
 * Manifest content is immutable per path, so no TTL needed.
 */
export async function fetchManifestContent(manifestPath: string): Promise<string> {
  const cacheKey = `${WINGET_CACHE_PREFIX}/files/${manifestPath}`;

  const cached = await cacheStorage.getItem(cacheKey);
  if (cached && typeof cached === "string") {
    return cached;
  }

  const rawUrl = `${WINGET_GITHUB_RAW_BASE}/${manifestPath}`;
  const response = await fetch(rawUrl);

  if (!response.ok) {
    throw new Error(`Failed to fetch manifest: ${response.statusText}`);
  }

  const content = await response.text();
  await cacheStorage.setItem(cacheKey, content);

  return content;
}

/**
 * Get ALL manifest file paths for a specific version.
 * Uses cached GitHub tree paths to discover files dynamically.
 */
export async function getVersionManifests(
  packageId: WinGetPackageIdentifier,
  version: WinGetPackageVersion,
): Promise<string[]> {
  const parts = packageId.split(".");
  if (parts.length < 2) return [];

  const firstChar = parts[0]?.[0];
  if (!firstChar) return [];

  const letter = firstChar.toLowerCase();
  const letterShas = await getLetterDirectoryShas();
  const sha = letterShas.get(letter);
  if (!sha) return [];

  const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);

  const publisher = parts[0];
  const name = parts.slice(1).join("/");
  if (!publisher || !name) return [];

  const pathPrefix = `${publisher}/${name}/${version}/`;

  return paths
    .filter((path) => path.startsWith(pathPrefix) && path.endsWith(".yaml"))
    .map((path) => `manifests/${letter}/${path}`);
}

/**
 * Build a WinGetVersionManifest by fetching and merging all manifest files
 * (main, installer, locale) for a given package version.
 */
export async function buildVersionManifest(
  packageId: WinGetPackageIdentifier,
  version: WinGetPackageVersion,
): Promise<WinGetVersionManifest | null> {
  const manifestFiles = await getVersionManifests(packageId, version);
  if (manifestFiles.length === 0) return null;

  // Fetch all manifest files in parallel
  const fetched = await Promise.allSettled(
    manifestFiles.map(async (manifestPath) => {
      const content = await fetchManifestContent(manifestPath);
      return {
        filename: manifestPath.split("/").pop()!,
        manifest: parseYAML(content) as Record<string, any>,
      };
    }),
  );

  const versionEntry: WinGetVersionManifest = { PackageVersion: version };

  for (const result of fetched) {
    if (result.status !== "fulfilled") continue;
    const { filename, manifest } = result.value;

    if (filename === `${packageId}.yaml`) {
      versionEntry.DefaultLocale = manifest.DefaultLocale;
      versionEntry.Channel = manifest.Channel;
      // Inline locale data when no dedicated locale file exists
      const hasLocaleData = Boolean(
        manifest.PackageLocale || manifest.Publisher || manifest.PackageName,
      );
      const defaultLocale = manifest.DefaultLocale || manifest.PackageLocale;
      const hasDefaultLocaleFile = defaultLocale
        ? manifestFiles.some((p) => p.includes(`.locale.${defaultLocale}.yaml`))
        : false;
      if (hasLocaleData && !hasDefaultLocaleFile) {
        if (!versionEntry.Locales) versionEntry.Locales = [];
        versionEntry.Locales.unshift({
          PackageLocale: defaultLocale || manifest.PackageLocale,
          ...manifest,
        } as Record<string, any>);
      }
    } else if (filename.endsWith(".installer.yaml")) {
      if (manifest.Installers && Array.isArray(manifest.Installers)) {
        versionEntry.Installers = manifest.Installers.map((inst: Record<string, any>) => ({
          ...manifest,
          ...inst,
        }));
      }
    } else if (/\.locale\.[^.]+\.yaml$/.test(filename)) {
      if (!versionEntry.Locales) versionEntry.Locales = [];
      versionEntry.Locales.push(manifest);
    }
  }

  return versionEntry;
}
