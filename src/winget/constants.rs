//! WinGet GitHub manifest source constants (mirrors winget/constants.ts).

/// GitHub repository hosting the WinGet community manifests.
pub const WINGET_GITHUB_REPO: &str = "microsoft/winget-pkgs";

/// Default branch.
pub const WINGET_GITHUB_BRANCH: &str = "master";

/// GitHub REST API base URL.
pub const WINGET_GITHUB_API_BASE: &str = "https://api.github.com";

/// GitHub raw content base URL for the manifests branch.
pub const WINGET_GITHUB_RAW_BASE: &str =
    "https://raw.githubusercontent.com/microsoft/winget-pkgs/master";

/// Cache key prefix for WinGet GitHub data.
pub const WINGET_CACHE_PREFIX: &str = "registry/winget/microsoft/winget-pkgs";

/// Cache key for the manifests directory SHA.
pub const WINGET_MANIFESTS_SHA_KEY: &str = "registry/winget/microsoft/winget-pkgs/manifests-sha";

/// Tree/SHA cache TTL in seconds (10 minutes).
pub const WINGET_UPDATE_INTERVAL_SECS: i64 = 600;

/// Page size for the versions endpoint.
pub const WINGET_VERSIONS_PAGE_SIZE: usize = 25;

/// Page size for the installers endpoint.
pub const WINGET_INSTALLERS_PAGE_SIZE: usize = 25;

/// Page size for the locales endpoint.
pub const WINGET_LOCALES_PAGE_SIZE: usize = 25;

/// Max concurrent version-manifest builds for the packageManifests endpoint. Each
/// build fans out to a few file fetches; those are themselves capped by
/// DOWNLOAD_SEMAPHORE, so this bounds only the number of builds in flight.
pub const WINGET_MANIFEST_BUILD_CONCURRENCY: usize = 16;

/// Default locale used when a manifest omits one.
pub const WINGET_DEFAULT_LOCALE: &str = "en-US";
