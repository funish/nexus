//! WinGet REST Source request/response types and response builders.
//! Fields serialize as PascalCase to match the WinGet client contract.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

/// Match type for search queries (WinGet REST Source spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
#[derive(Default)]
pub enum MatchType {
    Exact,
    #[default]
    CaseInsensitive,
    StartsWith,
    Substring,
    Wildcard,
    Fuzzy,
    FuzzySubstring,
}

/// Package match field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PackageMatchField {
    PackageIdentifier,
    PackageName,
    Publisher,
    Moniker,
    Command,
    Tag,
    PackageFamilyName,
    ProductCode,
    UpgradeCode,
    NormalizedPackageNameAndPublisher,
    Market,
    HasInstallerType,
}

/// Request match (Query or Filter.RequestMatch).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct SearchRequestMatch {
    pub key_word: Option<String>,
    pub match_type: Option<MatchType>,
    #[allow(dead_code)]
    pub package_match_field: Option<PackageMatchField>,
}

/// Inclusions/Filters element.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageMatchFilter {
    pub package_match_field: PackageMatchField,
    pub request_match: SearchRequestMatch,
}

/// manifestSearch POST request body.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct ManifestSearchRequest {
    pub maximum_results: Option<usize>,
    #[allow(dead_code)]
    pub fetch_all_manifests: Option<bool>,
    #[serde(default)]
    pub query: Option<SearchRequestMatch>,
    #[serde(default)]
    pub inclusions: Option<Vec<PackageMatchFilter>>,
    #[serde(default)]
    pub filters: Option<Vec<PackageMatchFilter>>,
}

/// Version (shared by the search index internals and the output).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ManifestVersion {
    pub package_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Single search result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ManifestSearchResult {
    pub package_identifier: String,
    pub package_name: String,
    pub publisher: String,
    pub versions: Vec<ManifestVersion>,
}

/// manifestSearch response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ManifestSearchResponse {
    pub data: Vec<ManifestSearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
    pub required_package_match_fields: Vec<String>,
    pub unsupported_package_match_fields: Vec<String>,
}

/// WinGet error item (the response is an array of these).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct WinGetErrorItem {
    pub error_code: u16,
    pub error_message: String,
}

/// Single element of the packages endpoint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageIdentifierItem {
    pub package_identifier: String,
}

/// packages endpoint response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackagesResponse {
    pub data: Vec<PackageIdentifierItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// GET /information response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InformationResponse {
    pub data: InformationData,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InformationData {
    pub source_identifier: String,
    pub server_supported_versions: Vec<String>,
    pub required_package_match_fields: Vec<String>,
    pub unsupported_package_match_fields: Vec<String>,
    pub unsupported_query_parameters: Vec<String>,
    pub required_query_parameters: Vec<String>,
    pub authentication: AuthenticationInfo,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticationInfo {
    pub authentication_type: String,
}

/// GET /packages/:id response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageSingleResponse {
    pub data: PackageIdentifierItem,
}

// ── Version manifest (packageManifests endpoint) ──────────────────────────

/// Internal merged version manifest (mirrors WinGetVersionManifest).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct VersionManifest {
    pub package_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installers: Option<Vec<serde_json::Value>>,
}

/// GET /packageManifests/:id response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageManifestResponse {
    pub data: PackageManifestData,
    pub unsupported_query_parameters: Vec<String>,
    pub required_query_parameters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageManifestData {
    pub package_identifier: String,
    pub versions: Vec<VersionManifest>,
}

// ── Versions endpoint ─────────────────────────────────────────────────────

/// Versions endpoint item (mirrors WinGetVersionSchema).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct VersionSchema {
    pub package_version: String,
    pub default_locale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// GET /packages/:id/versions response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct VersionMultipleResponse {
    pub data: Vec<VersionSchema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

// ── Installers endpoint ───────────────────────────────────────────────────

/// GET /packages/:id/versions/:version/installers response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InstallerMultipleResponse {
    pub data: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// GET .../installers/:installer response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InstallerSingleResponse {
    pub data: serde_json::Value,
}

// ── Locales endpoint ──────────────────────────────────────────────────────

/// GET /packages/:id/versions/:version/locales response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocaleMultipleResponse {
    pub data: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// GET .../locales/:locale response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocaleSingleResponse {
    pub data: serde_json::Value,
}

// ── Response builders ──────────────────────────────────────────────────────

/// Serialize a value as a 200 JSON response.
pub fn json_ok<T: Serialize>(body: &T) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Build a WinGet-spec error response: `[{ ErrorCode, ErrorMessage }]`.
pub fn winget_error(status: StatusCode, message: &str) -> Response {
    let body = vec![WinGetErrorItem {
        error_code: status.as_u16(),
        error_message: message.to_string(),
    }];
    (
        status,
        [("content-type", "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "[]".to_string()),
    )
        .into_response()
}
