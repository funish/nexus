import semver from "semver";

import { CDN_CACHE_LONG, CDN_CACHE_SHORT } from "./constants";

/**
 * Get appropriate Cache-Control header based on version.
 * Non-semver versions (branches, refs) get short cache; complete semver gets long cache.
 */
export function getCacheControl(version: string): string {
  return semver.valid(version) ? CDN_CACHE_LONG : CDN_CACHE_SHORT;
}
