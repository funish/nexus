/**
 * WinGet API types and utilities
 * Based on WinGet.RestSource OpenAPI specification v1.9.0
 */

// GitHub Tree API types

/**
 * GitHub Tree API response item
 */
export interface WinGetGitHubTreeItem {
  path: string;
  mode: string;
  type: "tree" | "blob";
  sha: string;
  size?: number;
  url: string;
}

/**
 * GitHub Tree API response
 */
export interface WinGetGitHubTreeResponse {
  sha: string;
  url: string;
  tree: WinGetGitHubTreeItem[];
  truncated: boolean;
}

// WinGet Core types

/**
 * Package identifier in WinGet format
 * Example: Microsoft.VisualStudioCode
 */
export type WinGetPackageIdentifier = string;

/**
 * Package version
 * Example: 1.95.0
 */
export type WinGetPackageVersion = string;

/**
 * WinGet package metadata (PackageSchema)
 */
export interface WinGetPackage {
  PackageIdentifier: WinGetPackageIdentifier;
}

/**
 * Single package response
 */
export interface WinGetPackageSingleResponse {
  Data: {
    PackageIdentifier: WinGetPackageIdentifier;
  };
}

/**
 * Multiple packages response (with pagination)
 */
export interface WinGetPackageMultipleResponse {
  Data: WinGetPackage[];
  ContinuationToken?: string;
}

// Version types

/**
 * Version Schema (WinGet 1.9.0)
 */
export interface WinGetVersionSchema {
  PackageVersion: WinGetPackageVersion;
  DefaultLocale: string;
  Channel?: string;
}

/**
 * Single version response (WinGet 1.9.0)
 */
export interface WinGetVersionSingleResponse {
  Data: WinGetVersionSchema;
}

/**
 * Internal version manifest representation (used by packageManifests endpoint)
 */
export interface WinGetVersionManifest {
  PackageVersion: string;
  DefaultLocale?: string;
  Channel?: string | null;
  Locales?: Record<string, any>[];
  Installers?: Record<string, any>[];
}

/**
 * Multiple versions response (WinGet 1.9.0)
 */
export interface WinGetVersionMultipleResponse {
  Data: WinGetVersionSchema[];
  ContinuationToken?: string;
}

// Locale types

/**
 * Locale Schema (WinGet 1.9.0)
 */
export interface WinGetLocaleSchema {
  PackageLocale: string;
  [key: string]: any;
}

/**
 * Single locale response (WinGet 1.9.0)
 */
export interface WinGetLocaleSingleResponse {
  Data: WinGetLocaleSchema;
}

/**
 * Multiple locales response (WinGet 1.9.0)
 */
export interface WinGetLocaleMultipleResponse {
  Data: WinGetLocaleSchema[];
  ContinuationToken?: string;
}

// Installer types

/**
 * Installer Schema (WinGet 1.9.0)
 */
export interface WinGetInstallerSchema {
  [key: string]: any;
}

/**
 * Single installer response (WinGet 1.9.0)
 */
export interface WinGetInstallerSingleResponse {
  Data: WinGetInstallerSchema;
}

/**
 * Multiple installers response (WinGet 1.9.0)
 */
export interface WinGetInstallerMultipleResponse {
  Data: WinGetInstallerSchema[];
  ContinuationToken?: string;
}

// Error types

/**
 * Error response (WinGet REST Source format: array)
 */
export interface WinGetError {
  ErrorCode: number;
  ErrorMessage: string;
}

// Search types

/**
 * Lightweight entry for fuse.js search index.
 * Extracted from index.db and cached in memoryStorage for fast fuzzy search.
 * Contains all fields needed to serve search responses without a second DB query.
 */
export interface WinGetSearchEntry {
  id: string;
  name: string;
  publisher: string;
  monikers: string[];
  tags: string[];
  commands: string[];
  versions: WinGetManifestSearchVersionResponse[];
  packageFamilyNames: string[];
  productCodes: string[];
  upgradeCodes: string[];
}

/**
 * Match type for search queries
 */
export type WinGetMatchType =
  | "Exact"
  | "CaseInsensitive"
  | "StartsWith"
  | "Substring"
  | "Wildcard"
  | "Fuzzy"
  | "FuzzySubstring";

/**
 * Package match field for search
 */
export type WinGetPackageMatchField =
  | "PackageIdentifier"
  | "PackageName"
  | "Moniker"
  | "Command"
  | "Tag"
  | "PackageFamilyName"
  | "ProductCode"
  | "UpgradeCode"
  | "NormalizedPackageNameAndPublisher"
  | "Market"
  | "HasInstallerType";

/**
 * Search request match
 */
export interface WinGetSearchRequestMatch {
  KeyWord?: string;
  MatchType?: WinGetMatchType;
  PackageMatchField?: WinGetPackageMatchField;
}

/**
 * Search request package match filter (Inclusions/Filters item)
 */
export interface WinGetSearchRequestPackageMatchFilter {
  PackageMatchField: WinGetPackageMatchField;
  RequestMatch: WinGetSearchRequestMatch;
}

/**
 * Manifest search request
 */
export interface WinGetManifestSearchRequest {
  MaximumResults?: number;
  FetchAllManifests?: boolean;
  Query?: WinGetSearchRequestMatch;
  Inclusions?: WinGetSearchRequestPackageMatchFilter[];
  Filters?: WinGetSearchRequestPackageMatchFilter[];
}

/**
 * Manifest search version response
 */
export interface WinGetManifestSearchVersionResponse {
  PackageVersion: WinGetPackageVersion;
  Channel?: string;
  PackageFamilyNames?: string[];
  ProductCodes?: string[];
  AppsAndFeaturesEntryVersions?: string[];
  UpgradeCodes?: string[];
}

/**
 * Manifest search response
 */
export interface WinGetManifestSearchResponse {
  PackageIdentifier: WinGetPackageIdentifier;
  PackageName: string;
  Publisher: string;
  Versions: WinGetManifestSearchVersionResponse[];
}

/**
 * Manifest search result
 */
export interface WinGetManifestSearchResult {
  Data: WinGetManifestSearchResponse[];
  ContinuationToken?: string;
  RequiredPackageMatchFields?: WinGetPackageMatchField[];
  UnsupportedPackageMatchFields?: WinGetPackageMatchField[];
}
