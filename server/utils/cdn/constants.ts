/** Tarball download timeout in milliseconds (10 seconds) */
export const TARBALL_DOWNLOAD_TIMEOUT = 10_000;

/** Short cache duration for mutable/branch/incomplete versions (10 minutes) */
export const CACHE_CONTROL_SHORT = "public, max-age=600";

/** Long cache duration for immutable/complete semver versions (1 year) */
export const CACHE_CONTROL_LONG = "public, max-age=31536000, immutable";

/** Maximum allowed unpacked package size in bytes (50 MB) */
export const MAX_UNPACKED_SIZE = 50 * 1024 * 1024;
