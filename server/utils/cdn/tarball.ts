import { HTTPError } from "nitro/h3";

import { cacheStorage } from "../storage";
import { MAX_UNPACKED_SIZE, SKIP_TTL_MS, TARBALL_DOWNLOAD_TIMEOUT } from "./constants";
import { calculateIntegrity } from "./integrity";
import type { CdnFile } from "./types";

// Concurrency guard: prevent duplicate tarball downloads for the same cacheBase
const pendingTarballs = new Set<string>();

/**
 * Options for tarball root directory detection.
 */
export interface RootDirOptions {
  /**
   * If true, skip entries starting with 'pax_global_header' when finding
   * the root directory. Needed for GitHub tarballs which include
   * Pax header entries.
   * Default: false
   */
  skipPaxHeaders?: boolean;

  /**
   * Fallback directory name if root cannot be detected from tarball entries.
   * Default: "package"
   */
  fallbackName?: string;
}

/**
 * Detect the root directory name inside a tarball's file map.
 */
export function detectRootDir(filesMap: Map<string, File>, options?: RootDirOptions): string {
  const keys = [...filesMap.keys()];
  let firstPath: string | undefined;

  if (options?.skipPaxHeaders) {
    firstPath = keys.find((p) => p.includes("/") && !p.startsWith("pax_global_header"));
  } else {
    firstPath = keys[0];
  }

  const fallback = options?.fallbackName ?? "package";
  return firstPath?.split("/")[0] || fallback;
}

/**
 * Download a tarball with timeout and return its bytes.
 * Throws HTTPError on timeout or non-OK response.
 */
export async function downloadTarball(
  tarballUrl: string,
  errorContext = "package",
): Promise<Uint8Array> {
  let tarballRes: Response;
  try {
    tarballRes = await fetch(tarballUrl, { signal: AbortSignal.timeout(TARBALL_DOWNLOAD_TIMEOUT) });
  } catch (error) {
    if ((error as Error).name === "AbortError") {
      throw new HTTPError({
        status: 504,
        statusText: `Tarball download timed out, ${errorContext} may be too large`,
      });
    }
    throw new HTTPError({ status: 502, statusText: "Failed to download tarball" });
  }

  if (!tarballRes.ok) {
    throw new HTTPError({ status: 502, statusText: "Failed to download tarball" });
  }

  return tarballRes.bytes();
}

/**
 * Try fetching a URL with timeout and return bytes if successful.
 * Returns undefined on any failure (network error, non-OK status, timeout).
 */
async function tryFetchWithTimeout(url: string): Promise<Uint8Array | undefined> {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(TARBALL_DOWNLOAD_TIMEOUT) });
    if (res.ok) {
      return res.bytes();
    }
  } catch {
    // Ignore network errors, timeouts, and non-OK responses
  }
  return undefined;
}

/**
 * Extract a single file from a tarball (cache-first, download on miss).
 *
 * If `directUrl` is provided, it will be tried before downloading the tarball.
 * Fallback chain for GitHub raw URLs: raw → raw/master → jsdelivr CDN → tarball.
 */
export async function extractFileFromTarball(
  tarballUrl: string,
  filepath: string,
  cacheKey: string,
  rootDirOptions?: RootDirOptions,
  directUrl?: string,
): Promise<Uint8Array> {
  const storage = cacheStorage;

  // Try cache first
  const cached = await storage.getItemRaw(cacheKey);
  if (cached) {
    return new Uint8Array(cached);
  }

  // Try direct URL and fallbacks
  if (directUrl) {
    // 1. Try raw.githubusercontent.com
    const data = await tryFetchWithTimeout(directUrl);
    if (data) {
      await storage.setItemRaw(cacheKey, data);
      return data;
    }

    // 2. If default main was used, try master and jsdelivr CDN
    if (directUrl.includes("/main/")) {
      const masterUrl = directUrl.replace(/\/main\//, "/master/");
      const masterData = await tryFetchWithTimeout(masterUrl);
      if (masterData) {
        await storage.setItemRaw(cacheKey, masterData);
        return masterData;
      }

      // jsdelivr CDN without version (resolves default branch automatically)
      const jsdelivrBase = directUrl.replace(
        "https://raw.githubusercontent.com/",
        "https://cdn.jsdelivr.net/gh/",
      );
      const urlObj = new URL(jsdelivrBase);
      const pathParts = urlObj.pathname.split("/");
      if (pathParts.length >= 6) {
        pathParts.splice(4, 1);
      }
      urlObj.pathname = pathParts.join("/");
      const jsdelivrData = await tryFetchWithTimeout(urlObj.toString());
      if (jsdelivrData) {
        await storage.setItemRaw(cacheKey, jsdelivrData);
        return jsdelivrData;
      }
    }
  }

  // Download tarball and extract the target file
  const tarballData = await downloadTarball(tarballUrl);
  const archive = new Bun.Archive(tarballData);

  const rootDir = await detectRootDirFromArchive(archive, rootDirOptions);
  const globPattern = `${rootDir}/${filepath}`;
  const matchedFiles = await archive.files(globPattern);
  const targetFile = matchedFiles.get(globPattern);

  if (!targetFile) {
    throw new HTTPError({
      status: 404,
      statusText: `File not found: ${filepath}`,
    });
  }

  return new Uint8Array(await targetFile.bytes());
}

/**
 * Detect root directory from a Bun.Archive without listing all files.
 */
async function detectRootDirFromArchive(
  archive: Bun.Archive,
  options?: RootDirOptions,
): Promise<string> {
  const filesMap = await archive.files();
  return detectRootDir(filesMap, options);
}

/**
 * Check if a package version's file list is already cached.
 */
export async function isPackageCached(
  cacheBase: string,
  isCacheableVersion: boolean,
): Promise<boolean> {
  if (!isCacheableVersion) return false;
  const storage = cacheStorage;
  const meta = await storage.getMeta(cacheBase);
  return !!meta?.files;
}

/**
 * Download and cache all files from a tarball.
 * Intended for background execution (via event.waitUntil or after response).
 */
export async function cachePackageFromTarball(
  tarballUrl: string,
  cacheBase: string,
  rootDirOptions?: RootDirOptions,
  logLabel?: string,
): Promise<void> {
  const storage = cacheStorage;

  // Check if already cached with file list, or still within skip TTL
  const cachedMeta = await storage.getMeta(cacheBase);
  if (cachedMeta?.files) return;
  if (cachedMeta?.skippedAt && Date.now() - Number(cachedMeta.skippedAt) < SKIP_TTL_MS) return;

  // Skip if another call is already downloading this tarball
  if (pendingTarballs.has(cacheBase)) return;
  pendingTarballs.add(cacheBase);

  try {
    // Download and extract tarball
    const tarballRes = await fetch(tarballUrl, {
      signal: AbortSignal.timeout(TARBALL_DOWNLOAD_TIMEOUT),
    });
    if (!tarballRes.ok) {
      console.error(`Failed to download tarball for ${logLabel || cacheBase}`);
      await storage.setMeta(cacheBase, { skippedAt: Date.now() });
      return;
    }

    const tarballData = await tarballRes.bytes();
    const archive = new Bun.Archive(tarballData);
    const filesMap = await archive.files();

    const rootDir = detectRootDir(filesMap, rootDirOptions);
    const rootPath = `${rootDir}/`;

    // Build file list metadata
    const fileList: Array<CdnFile> = [];
    for (const [path, file] of filesMap) {
      fileList.push({
        name: path.slice(rootPath.length),
        size: file.size,
      });
    }

    // Skip packages that exceed size limit
    const totalSize = fileList.reduce((sum, f) => sum + f.size, 0);
    if (totalSize > MAX_UNPACKED_SIZE) {
      console.warn(
        `Skipping ${logLabel || cacheBase}: unpacked size ${(totalSize / 1024 / 1024).toFixed(1)} MB exceeds ${MAX_UNPACKED_SIZE / 1024 / 1024} MB limit`,
      );
      await storage.setMeta(cacheBase, { skippedAt: Date.now() });
      return;
    }

    // Concurrent caching using Promise.allSettled
    const cachePromises = Array.from(filesMap.entries()).map(async ([path, file]) => {
      const relativePath = path.slice(rootPath.length);
      const cacheKey = `${cacheBase}/${relativePath}`;

      try {
        const exists = await storage.getItemRaw(cacheKey);
        if (exists) return;

        const fileData = await file.bytes();
        await storage.setItemRaw(cacheKey, fileData);

        // Calculate SHA-256 integrity for SRI
        const integrity = calculateIntegrity(fileData);
        const fileItem = fileList.find((f) => f.name === relativePath);
        if (fileItem) {
          fileItem.integrity = integrity;
        }
      } catch (error) {
        console.error(`Failed to cache file ${relativePath}:`, error);
      }
    });

    await Promise.allSettled(cachePromises);

    // Store file list in meta
    await storage.setMeta(cacheBase, { files: fileList });
  } catch (error) {
    console.error(`Background cache failed for ${logLabel || cacheBase}:`, error);
  } finally {
    pendingTarballs.delete(cacheBase);
  }
}
