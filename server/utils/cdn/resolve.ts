import semver from "semver";

import type { CdnRegistryMetadata, CdnResolvedVersion } from "./types";

/**
 * Resolve a requested version string against npm-style registry metadata.
 *
 * Resolution order:
 *  1. Exact match in `metadata.versions[version]`
 *  2. Semver range match via `semver.maxSatisfying`
 *  3. Fallback to `dist-tags.latest`
 *
 * Returns `null` when no matching version is found.
 */
export function resolveRegistryVersion(
  metadata: CdnRegistryMetadata,
  requestedVersion: string,
): CdnResolvedVersion | null {
  const distTags = metadata["dist-tags"];
  let versionInfo = metadata.versions[requestedVersion];
  let version = requestedVersion;

  // Try semver range matching when exact version is absent
  if (!versionInfo) {
    const allVersions = Object.keys(metadata.versions);
    const matchedVersion = semver.maxSatisfying(allVersions, version);
    if (matchedVersion) {
      version = matchedVersion;
      versionInfo = metadata.versions[version];
    }
  }

  // Fallback to latest dist-tag
  if (!versionInfo && distTags?.latest) {
    const latest = distTags.latest;
    if (latest) {
      versionInfo = metadata.versions[latest];
      version = latest;
    }
  }

  if (!versionInfo) return null;
  return { version, versionInfo };
}
