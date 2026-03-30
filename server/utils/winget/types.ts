/**
 * WinGet API types and utilities
 * Based on WinGet.RestSource OpenAPI specification v1.9.0
 */

// GitHub Tree API types

/**
 * GitHub Tree API response item
 */
export interface GitHubTreeItem {
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
export interface GitHubTreeResponse {
  sha: string;
  url: string;
  tree: GitHubTreeItem[];
  truncated: boolean;
}

// WinGet Core types

/**
 * Package identifier in WinGet format
 * Example: Microsoft.VisualStudioCode
 */
export type PackageIdentifier = string;

/**
 * Package version
 * Example: 1.95.0
 */
export type PackageVersion = string;

/**
 * WinGet package metadata (PackageSchema)
 */
export interface WinGetPackage {
  PackageIdentifier: PackageIdentifier;
}

/**
 * Single package response
 */
export interface PackageSingleResponse {
  Data: {
    PackageIdentifier: PackageIdentifier;
  };
}

/**
 * Multiple packages response (with pagination)
 */
export interface PackageMultipleResponse {
  Data: WinGetPackage[];
  ContinuationToken?: string;
}

// Version types

/**
 * Version Schema (WinGet 1.9.0)
 */
export interface VersionSchema {
  PackageVersion: PackageVersion;
  DefaultLocale: string;
  Channel?: string;
}

/**
 * Single version response (WinGet 1.9.0)
 */
export interface VersionSingleResponse {
  Data: VersionSchema;
}

/**
 * Internal version manifest representation (used by packageManifests endpoint)
 */
export interface VersionManifest {
  PackageVersion: string;
  DefaultLocale?: string;
  Channel?: string | null;
  Locales?: Record<string, any>[];
  Installers?: Record<string, any>[];
}

/**
 * Multiple versions response (WinGet 1.9.0)
 */
export interface VersionMultipleResponse {
  Data: VersionSchema[];
  ContinuationToken?: string;
}

// Locale types

/**
 * Locale Schema (WinGet 1.9.0)
 */
export interface LocaleSchema {
  PackageLocale: string;
  [key: string]: any;
}

/**
 * Single locale response (WinGet 1.9.0)
 */
export interface LocaleSingleResponse {
  Data: LocaleSchema;
}

/**
 * Multiple locales response (WinGet 1.9.0)
 */
export interface LocaleMultipleResponse {
  Data: LocaleSchema[];
  ContinuationToken?: string;
}

// Installer types

/**
 * Installer Schema (WinGet 1.9.0)
 */
export interface InstallerSchema {
  [key: string]: any;
}

/**
 * Single installer response (WinGet 1.9.0)
 */
export interface InstallerSingleResponse {
  Data: InstallerSchema;
}

/**
 * Multiple installers response (WinGet 1.9.0)
 */
export interface InstallerMultipleResponse {
  Data: InstallerSchema[];
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
 * Match type for search queries
 */
export type MatchType =
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
export type PackageMatchField =
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
export interface SearchRequestMatch {
  KeyWord?: string;
  MatchType?: MatchType;
  PackageMatchField?: PackageMatchField;
}

/**
 * Search request package match filter (Inclusions/Filters item)
 */
export interface SearchRequestPackageMatchFilter {
  PackageMatchField: PackageMatchField;
  RequestMatch: SearchRequestMatch;
}

/**
 * Manifest search request
 */
export interface ManifestSearchRequest {
  MaximumResults?: number;
  FetchAllManifests?: boolean;
  Query?: SearchRequestMatch;
  Inclusions?: SearchRequestPackageMatchFilter[];
  Filters?: SearchRequestPackageMatchFilter[];
}

/**
 * Manifest search version response
 */
export interface ManifestSearchVersionResponse {
  PackageVersion: PackageVersion;
  Channel?: string;
  PackageFamilyNames?: string[];
  ProductCodes?: string[];
  AppsAndFeaturesEntryVersions?: string[];
  UpgradeCodes?: string[];
}

/**
 * Manifest search response
 */
export interface ManifestSearchResponse {
  PackageIdentifier: PackageIdentifier;
  PackageName: string;
  Publisher: string;
  Versions: ManifestSearchVersionResponse[];
  /** @internal fuzzy search score, removed before response */
  _fuzzyScore?: number;
}

/**
 * Manifest search result
 */
export interface ManifestSearchResult {
  Data: ManifestSearchResponse[];
  ContinuationToken?: string;
  RequiredPackageMatchFields?: PackageMatchField[];
  UnsupportedPackageMatchFields?: PackageMatchField[];
}
