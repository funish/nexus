/** Tarball download timeout in milliseconds (30 seconds) */
export const TARBALL_DOWNLOAD_TIMEOUT = 30_000;

/** Short cache duration for mutable/branch/incomplete versions (10 minutes) */
export const CACHE_CONTROL_SHORT = "public, max-age=600";

/** Long cache duration for immutable/complete semver versions (1 year) */
export const CACHE_CONTROL_LONG = "public, max-age=31536000, immutable";

/** How long a skipped package stays skipped before retry (10 minutes) */
export const SKIP_TTL_MS = 10 * 60 * 1000;

/** npm registry base URL */
export const NPM_REGISTRY_URL = "https://registry.npmjs.org";

/** JSR registry base URL (npm compatibility layer) */
export const JSR_REGISTRY_URL = "https://npm.jsr.io";

/** Maximum allowed unpacked package size in bytes (50 MB) */
export const MAX_UNPACKED_SIZE = 50 * 1024 * 1024;
