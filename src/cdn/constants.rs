use std::env;
use std::sync::OnceLock;

pub const CDN_FETCH_TIMEOUT_SECS: u64 = 15;

fn configured_url(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| default.to_string())
}

/// npm registry base URL (package metadata, org listings).
pub fn npm_registry() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| configured_url("NPM_REGISTRY_URL", "https://registry.npmjs.org"))
}

/// JSR npm-compatible registry base URL (package metadata).
pub fn jsr_registry() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| configured_url("JSR_REGISTRY_URL", "https://npm.jsr.io"))
}

/// GitHub REST API base URL (repo tags for version resolution).
pub fn github_api_base() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| configured_url("GITHUB_API_BASE_URL", "https://api.github.com"))
}

/// cdnjs API base URL (library / version metadata).
pub fn cdnjs_api_base() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| configured_url("CDNJS_API_BASE_URL", "https://api.cdnjs.com"))
}

pub const CDN_CACHE_SHORT: &str = "public, max-age=600, s-maxage=600"; // listing/org responses
pub const CDN_CACHE_LONG: &str = "public, max-age=31536000, s-maxage=31536000, immutable"; // exact version/commit, 1yr
pub const CDN_CACHE_TAG: &str = "public, max-age=604800, s-maxage=43200"; // tag/latest alias: 7d browser / 12h CDN edge (matches jsDelivr)
pub const CDN_CACHE_BRANCH: &str = "public, max-age=43200, s-maxage=43200"; // branch ref, 12h (jsDelivr)
pub const CDN_SKIP_TTL_MS: u64 = 600_000;
pub const CDN_MAX_PACKAGE_SIZE: u64 = 50 * 1024 * 1024;
/// Server-side storage TTL for mutable refs (branches, re-pointable tags): 12h,
/// matching the CDN_CACHE_BRANCH edge TTL. Immutable exact versions/commits skip
/// this check entirely.
pub const CDN_MUTABLE_REF_TTL_SECS: i64 = 43_200;
