/**
 * CDN unified types
 */

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
 *
 * @example
 * Accessing /cdn/npm/uikit/dist/ returns:
 * { path: "dist", files: [...] }
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
