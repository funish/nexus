// ── Network ──────────────────────────────────────────────

/** CDN fetch timeout (15 seconds) */
export const CDN_FETCH_TIMEOUT = 15_000;

// ── Registry URLs ───────────────────────────────────────

/** npm registry base URL */
export const CDN_NPM_REGISTRY = "https://registry.npmjs.org";

/** JSR registry base URL (npm compatibility layer) */
export const CDN_JSR_REGISTRY = "https://npm.jsr.io";

// ── Cache Strategy ──────────────────────────────────────

/** Short cache for mutable/branch/incomplete versions (10 minutes) */
export const CDN_CACHE_SHORT = "public, max-age=600";

/** Long cache for immutable/complete semver versions (1 year) */
export const CDN_CACHE_LONG = "public, max-age=31536000, immutable";

/** How long a skipped package stays skipped before retry (10 minutes) */
export const CDN_SKIP_TTL = 10 * 60 * 1000;

// ── Limits ──────────────────────────────────────────────

/** Maximum unpacked package size in bytes (50 MB) */
export const CDN_MAX_PACKAGE_SIZE = 50 * 1024 * 1024;
