import semver from "semver";

import { CACHE_CONTROL_LONG, CACHE_CONTROL_SHORT } from "./constants";

/**
 * Get appropriate Cache-Control header based on version.
 * Non-semver versions (branches, refs) get short cache; complete semver gets long cache.
 */
export function getCacheControl(version: string): string {
  return semver.valid(version) ? CACHE_CONTROL_LONG : CACHE_CONTROL_SHORT;
}
