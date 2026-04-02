/**
 * File item in directory listing
 */
export interface CdnFile {
  name: string;
  size: number;
  integrity?: string;
}

/**
 * Directory listing response
 * All CDN endpoints return this format for directory listings
 */
export interface CdnDirectoryListing {
  path: string;
  files: CdnFile[];
}

/**
 * Enhanced package listing with metadata
 * Flattened structure combining directory listing with package information
 */
export interface CdnPackageListing extends CdnDirectoryListing {
  name?: string;
  version?: string;
}

/**
 * Organization package listing
 * Returned when accessing a bare scope (e.g., /cdn/npm/@vue)
 */
export interface CdnOrgListing {
  name: string;
  packages: string[];
}

/**
 * Minimal npm-registry-compatible metadata shape.
 * Both npm and JSR registries expose this structure.
 */
export interface CdnRegistryMetadata {
  "dist-tags"?: Record<string, string>;
  versions: Record<string, any>;
}

/**
 * Resolved version result from registry metadata.
 */
export interface CdnResolvedVersion {
  version: string;
  versionInfo: any;
}

/**
 * Options for tarball root directory detection.
 */
export interface CdnRootDirOptions {
  /**
   * If true, skip entries starting with 'pax_global_header' when finding
   * the root directory. Needed for GitHub tarballs which include
   * Pax header entries.
   * Default: false
   */
  skipPaxHeaders?: boolean;

  /**
   * Fallback directory name if root cannot be detected from tarball entries.
   * Default: "package"
   */
  fallbackName?: string;
}

/**
 * Options for ESM package bundling.
 */
export interface CdnEsmBundleOptions {
  packageName: string;
  version: string;
  entryPoint: string;
}
