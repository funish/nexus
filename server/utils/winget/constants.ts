// ── GitHub ───────────────────────────────────────────────

/** GitHub repository for WinGet community manifests */
export const WINGET_GITHUB_REPO = "microsoft/winget-pkgs";

/** Default branch */
export const WINGET_GITHUB_BRANCH = "master";

/** GitHub API base URL */
export const WINGET_GITHUB_API_BASE = "https://api.github.com";

/** GitHub raw content base URL */
export const WINGET_GITHUB_RAW_BASE = `https://raw.githubusercontent.com/${WINGET_GITHUB_REPO}/${WINGET_GITHUB_BRANCH}`;

// ── Source Data ──────────────────────────────────────────

/** source.msix download URL for index.db */
export const WINGET_SOURCE_MSIX_URL = "https://cdn.winget.microsoft.com/cache/source.msix";

/** Cache update interval in seconds (10 minutes) */
export const WINGET_UPDATE_INTERVAL = 600;

// ── Cache Keys ───────────────────────────────────────────

/** Cache key prefix for WinGet data */
export const WINGET_CACHE_PREFIX = `registry/winget/${WINGET_GITHUB_REPO}`;

/** Cache key for index.db binary */
export const WINGET_INDEX_DB_KEY = `${WINGET_CACHE_PREFIX}/index.db`;

/** Cache key for manifests directory SHA */
export const WINGET_MANIFESTS_SHA_KEY = `${WINGET_CACHE_PREFIX}/manifests-sha`;

// ── Pagination ───────────────────────────────────────────

/** Default page size for package listing */
export const WINGET_PACKAGES_PAGE_SIZE = 100;

/** Default page size for version listing */
export const WINGET_VERSIONS_PAGE_SIZE = 25;

// ── Defaults ─────────────────────────────────────────────

/** Default locale used when manifest doesn't specify one */
export const WINGET_DEFAULT_LOCALE = "en-US";
