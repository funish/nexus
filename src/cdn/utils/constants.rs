pub const CDN_FETCH_TIMEOUT_SECS: u64 = 15;
pub const CDN_NPM_REGISTRY: &str = "https://registry.npmjs.org";
pub const CDN_JSR_REGISTRY: &str = "https://npm.jsr.io";
pub const CDN_CACHE_SHORT: &str = "public, max-age=600, s-maxage=600"; // listing/org responses
pub const CDN_CACHE_LONG: &str = "public, max-age=31536000, s-maxage=31536000, immutable"; // exact version/commit, 1yr
pub const CDN_CACHE_TAG: &str = "public, max-age=604800, s-maxage=604800"; // tag/latest alias, 7d (jsDelivr)
pub const CDN_CACHE_BRANCH: &str = "public, max-age=43200, s-maxage=43200"; // branch ref, 12h (jsDelivr)
pub const CDN_SKIP_TTL_MS: u64 = 600_000;
pub const CDN_MAX_PACKAGE_SIZE: u64 = 50 * 1024 * 1024;
