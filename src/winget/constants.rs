//! WinGet manifest source settings. The upstream database and GitHub repository are
//! configurable so private forks or mirrors can be served without code changes.

/// GitHub REST API base URL.
pub const WINGET_GITHUB_API_BASE: &str = "https://api.github.com";

/// GitHub raw content base URL for the configured repository and branch.
pub fn github_raw_base() -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{}",
        crate::config::winget_github_repo(),
        crate::config::winget_github_branch()
    )
}

/// Cache key prefix for the configured WinGet GitHub data.
pub fn cache_prefix() -> String {
    format!(
        "registry/winget/{}/{}",
        crate::config::winget_github_repo(),
        crate::config::winget_github_branch()
    )
}

/// Cache key for the manifests directory SHA.
pub fn manifests_sha_key() -> String {
    format!("{}/manifests-sha", cache_prefix())
}

/// Tree/SHA cache TTL in seconds (10 minutes).
pub const WINGET_UPDATE_INTERVAL_SECS: i64 = 600;

/// Edge cache directive for winget responses. winget data follows the 10-minute
/// index TTL above, so a 5-minute edge cache stays conservative and matches the
/// manifestSearch route.
pub const WINGET_EDGE_CACHE_CONTROL: &str = "public, max-age=300";

/// Page size for the versions endpoint.
pub const WINGET_VERSIONS_PAGE_SIZE: usize = 25;

/// Page size for the installers endpoint.
pub const WINGET_INSTALLERS_PAGE_SIZE: usize = 25;

/// Page size for the locales endpoint.
pub const WINGET_LOCALES_PAGE_SIZE: usize = 25;

/// Default locale used when a manifest omits one.
pub const WINGET_DEFAULT_LOCALE: &str = "en-US";
