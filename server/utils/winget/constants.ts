/** GitHub repository for WinGet community manifests */
export const GITHUB_REPO = "microsoft/winget-pkgs";

/** Default branch */
export const GITHUB_BRANCH = "master";

/** GitHub API base URL */
export const GITHUB_API_BASE = "https://api.github.com";

/** GitHub raw content base URL */
export const GITHUB_RAW_BASE = `https://raw.githubusercontent.com/${GITHUB_REPO}/${GITHUB_BRANCH}`;

/** Cache update interval in seconds (10 minutes) */
export const UPDATE_INTERVAL = 600;

/** Cache key prefix for WinGet data */
export const CACHE_PREFIX = `registry/winget/${GITHUB_REPO}`;

/** source.msix download URL for index.db */
export const SOURCE_MSIX_URL = "https://cdn.winget.microsoft.com/cache/source.msix";

/** Cache key for index.db binary */
export const INDEX_DB_KEY = `${CACHE_PREFIX}/index.db`;

/** Cache key for manifests directory SHA */
export const MANIFESTS_SHA_KEY = `${CACHE_PREFIX}/manifests-sha`;
