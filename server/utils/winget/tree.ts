import { env } from "std-env";

import { cacheStorage } from "../storage";
import {
  WINGET_CACHE_PREFIX,
  WINGET_GITHUB_API_BASE,
  WINGET_GITHUB_BRANCH,
  WINGET_GITHUB_REPO,
  WINGET_MANIFESTS_SHA_KEY,
  WINGET_UPDATE_INTERVAL,
} from "./constants";
import type { WinGetGitHubTreeResponse } from "./types";

/**
 * Get GitHub authentication headers if token is available
 */
export function getGitHubHeaders(): HeadersInit {
  const token = env.GITHUB_TOKEN;

  const headers: HeadersInit = {
    "User-Agent": "Funish Nexus",
  };

  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  return headers;
}

/**
 * Fetch GitHub tree data by SHA or branch
 */
export async function getGitHubTree(
  treeSha: string = WINGET_GITHUB_BRANCH,
  recursive: boolean = false,
): Promise<WinGetGitHubTreeResponse> {
  const url = `${WINGET_GITHUB_API_BASE}/repos/${WINGET_GITHUB_REPO}/git/trees/${treeSha}${recursive ? "?recursive=1" : ""}`;
  const response = await fetch(url, {
    headers: getGitHubHeaders(),
  });

  if (!response.ok) {
    throw new Error(`Failed to fetch GitHub tree: ${response.statusText}`);
  }

  return (await response.json()) as WinGetGitHubTreeResponse;
}

/**
 * Fetch and cache GitHub tree paths (optimized storage)
 * @param treeSha - The SHA of the tree
 * @param cacheSuffix - Cache suffix for this tree (e.g. "manifests/m")
 * @returns Array of file paths
 */
export async function getGitHubTreePaths(treeSha: string, cacheSuffix: string): Promise<string[]> {
  const normalizedSuffix = cacheSuffix.replace(/\//g, "-");
  const cacheKey = `${WINGET_CACHE_PREFIX}/${normalizedSuffix}`;

  // Check cache metadata
  const meta = await cacheStorage.getMeta(cacheKey);
  const now = new Date();

  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < WINGET_UPDATE_INTERVAL) {
      const cached = await cacheStorage.getItem(cacheKey);
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
  await cacheStorage.setItem(cacheKey, paths);
  await cacheStorage.setMeta(cacheKey, { mtime: new Date() });

  return paths;
}

/**
 * Get the SHA of the manifests directory (cached 10-min TTL)
 */
export async function getManifestsSha(): Promise<string> {
  // Check cache metadata
  const meta = await cacheStorage.getMeta(WINGET_MANIFESTS_SHA_KEY);
  const now = new Date();

  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < WINGET_UPDATE_INTERVAL) {
      const cached = await cacheStorage.getItem(WINGET_MANIFESTS_SHA_KEY);
      if (cached && typeof cached === "string") {
        return cached;
      }
    }
  }

  // Fetch root tree
  const rootTree = await getGitHubTree(WINGET_GITHUB_BRANCH, false);
  const manifestsItem = rootTree.tree.find(
    (item) => item.path === "manifests" && item.type === "tree",
  );

  if (!manifestsItem) {
    throw new Error("manifests directory not found in repository");
  }

  // Cache the SHA
  await cacheStorage.setItem(WINGET_MANIFESTS_SHA_KEY, manifestsItem.sha);
  await cacheStorage.setMeta(WINGET_MANIFESTS_SHA_KEY, { mtime: new Date() });

  return manifestsItem.sha;
}

/**
 * Get all letter directory SHAs from manifests (a-z, 0-9)
 */
export async function getLetterDirectoryShas(): Promise<Map<string, string>> {
  const cacheKey = `${WINGET_CACHE_PREFIX}/letter-shas`;

  // Check cache
  const meta = await cacheStorage.getMeta(cacheKey);
  const now = new Date();

  if (meta?.mtime) {
    const cacheAge = (now.getTime() - new Date(meta.mtime).getTime()) / 1000;
    if (cacheAge < WINGET_UPDATE_INTERVAL) {
      const cached = await cacheStorage.getItem(cacheKey);
      if (cached) {
        return new Map(Object.entries(cached as Record<string, string>));
      }
    }
  }

  const manifestsSha = await getManifestsSha();
  const manifestsTree = await getGitHubTree(manifestsSha, false);

  const letterShas = new Map<string, string>();

  for (const item of manifestsTree.tree) {
    if (item.type === "tree" && item.path.length === 1 && /[a-z0-9]/.test(item.path)) {
      letterShas.set(item.path, item.sha);
    }
  }

  if (letterShas.size === 0) {
    throw new Error("No letter directories found in manifests");
  }

  // Cache as plain object (Map is not serializable)
  await cacheStorage.setItem(cacheKey, Object.fromEntries(letterShas));
  await cacheStorage.setMeta(cacheKey, { mtime: new Date() });

  return letterShas;
}
