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
