//! WinGet REST Source manifest endpoints (versions / installers / locales / packageManifests).
//!
//! These read package versions from index.db and assemble per-version manifests
//! from the winget-pkgs GitHub YAML files (mirrors the .backup route handlers).

use std::sync::LazyLock;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::storage::SharedStorage;
use crate::winget::utils::constants::*;
use crate::winget::utils::db::{SharedDb, get_index_db};
use crate::winget::utils::manifest::{
    ManifestType, build_version_manifest, construct_manifest_path, fetch_manifest_content,
    get_version_manifests, merge_installer, parse_yaml,
};
use crate::winget::utils::queries::get_package_versions;
use crate::winget::utils::response::{
    InstallerMultipleResponse, InstallerSingleResponse, LocaleMultipleResponse,
    LocaleSingleResponse, PackageManifestData, PackageManifestResponse, VersionManifest,
    VersionMultipleResponse, VersionSchema, json_ok, winget_error,
};
use crate::winget::utils::token::{decode_continuation_token, encode_continuation_token};

static LOCALE_FILE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.locale\.([^.]+)\.yaml$").unwrap());

type AppState = (SharedStorage, SharedDb);

/// Query params carrying a ContinuationToken.
#[derive(Deserialize)]
pub struct ContinuationParams {
    #[serde(rename = "ContinuationToken")]
    pub continuation_token: Option<String>,
}

/// Query params for the packageManifests endpoint.
#[derive(Deserialize)]
pub struct ManifestParams {
    #[serde(rename = "Version")]
    pub version: Option<String>,
    #[serde(rename = "Channel")]
    pub channel: Option<String>,
    #[serde(rename = "Market")]
    pub market: Option<String>,
}

/// GET /api/winget/packages/:id/versions
pub async fn handle_versions(
    State((storage, db)): State<AppState>,
    Path(package_id): Path<String>,
    Query(params): Query<ContinuationParams>,
) -> Response {
    let versions = match load_versions(&db, &storage, &package_id).await {
        Ok(v) => v,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if versions.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Package '{package_id}' not found"),
        );
    }

    // Fetch DefaultLocale from the latest version's main manifest.
    let mut default_locale = WINGET_DEFAULT_LOCALE.to_string();
    if let Some(latest) = versions.first() {
        let main_path = construct_manifest_path(&package_id, latest, ManifestType::Main, None);
        if let Ok(content) = fetch_manifest_content(&storage, &main_path).await
            && let Ok(manifest) = parse_yaml(&content)
            && let Some(dl) = manifest.get("DefaultLocale").and_then(|v| v.as_str())
        {
            default_locale = dl.to_string();
        }
    }

    let total = versions.len();
    let start = decode_continuation_token(params.continuation_token.as_deref()).min(total);
    let end = (start + WINGET_VERSIONS_PAGE_SIZE).min(total);
    let data: Vec<VersionSchema> = versions[start..end]
        .iter()
        .map(|v| VersionSchema {
            package_version: v.clone(),
            default_locale: default_locale.clone(),
            channel: None,
        })
        .collect();
    let continuation_token = (end < total).then(|| encode_continuation_token(end));
    json_ok(&VersionMultipleResponse {
        data,
        continuation_token,
    })
}

/// GET /api/winget/packages/:id/versions/:version/installers
pub async fn handle_installers(
    State((storage, _db)): State<AppState>,
    Path((package_id, version)): Path<(String, String)>,
    Query(params): Query<ContinuationParams>,
) -> Response {
    let files = match get_version_manifests(&storage, &package_id, &version).await {
        Ok(f) => f,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if files.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Version {version} of package '{package_id}' not found"),
        );
    }

    let installer_filename = format!("{package_id}.installer.yaml");
    let Some(installer_path) = file_by_basename(&files, &installer_filename) else {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!(
                "Installer manifest not found for version {version} of package '{package_id}'"
            ),
        );
    };

    let content = match fetch_manifest_content(&storage, installer_path).await {
        Ok(c) => c,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let manifest = match parse_yaml(&content) {
        Ok(m) => m,
        Err(e) => {
            return winget_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to parse installer manifest: {e}"),
            );
        }
    };

    let mut all_installers: Vec<Value> = Vec::new();
    if let Some(installers) = manifest.get("Installers").and_then(|v| v.as_array()) {
        for inst in installers {
            all_installers.push(merge_installer(&manifest, inst));
        }
    }

    let total = all_installers.len();
    let start = decode_continuation_token(params.continuation_token.as_deref()).min(total);
    let end = (start + WINGET_INSTALLERS_PAGE_SIZE).min(total);
    let data: Vec<Value> = all_installers[start..end].to_vec();
    let continuation_token = (end < total).then(|| encode_continuation_token(end));
    json_ok(&InstallerMultipleResponse {
        data,
        continuation_token,
    })
}

/// GET /api/winget/packages/:id/versions/:version/installers/:installer
pub async fn handle_installer(
    State((storage, _db)): State<AppState>,
    Path((package_id, version, installer_id)): Path<(String, String, String)>,
) -> Response {
    let files = match get_version_manifests(&storage, &package_id, &version).await {
        Ok(f) => f,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if files.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Version {version} of package '{package_id}' not found"),
        );
    }

    let installer_filename = format!("{package_id}.installer.yaml");
    let Some(installer_path) = file_by_basename(&files, &installer_filename) else {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!(
                "Installer manifest not found for version {version} of package '{package_id}'"
            ),
        );
    };

    let content = match fetch_manifest_content(&storage, installer_path).await {
        Ok(c) => c,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let manifest = match parse_yaml(&content) {
        Ok(m) => m,
        Err(e) => {
            return winget_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to parse installer manifest: {e}"),
            );
        }
    };

    let Some(installers) = manifest.get("Installers").and_then(|v| v.as_array()) else {
        return winget_error(StatusCode::NOT_FOUND, "No installers found in manifest");
    };
    let Some(installer) = installers.iter().find(|i| {
        i.get("InstallerIdentifier").and_then(|v| v.as_str()) == Some(installer_id.as_str())
    }) else {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Installer '{installer_id}' not found"),
        );
    };

    json_ok(&InstallerSingleResponse {
        data: merge_installer(&manifest, installer),
    })
}

/// GET /api/winget/packages/:id/versions/:version/locales
pub async fn handle_locales(
    State((storage, _db)): State<AppState>,
    Path((package_id, version)): Path<(String, String)>,
    Query(params): Query<ContinuationParams>,
) -> Response {
    let files = match get_version_manifests(&storage, &package_id, &version).await {
        Ok(f) => f,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if files.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Version {version} of package '{package_id}' not found"),
        );
    }

    let main_basename = format!("{package_id}.yaml");
    let main_manifest_file = file_by_basename(&files, &main_basename);
    let locale_files: Vec<&String> = files.iter().filter(|p| p.contains(".locale.")).collect();

    // Inline the default locale from the main manifest when no dedicated file exists.
    let mut main_locale_entry: Option<Value> = None;
    if let Some(main_path) = main_manifest_file
        && let Ok(content) = fetch_manifest_content(&storage, main_path).await
        && let Ok(manifest) = parse_yaml(&content)
    {
        let default_locale = manifest
            .get("DefaultLocale")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                manifest
                    .get("PackageLocale")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
        let has_locale_data = manifest
            .get("PackageLocale")
            .or(manifest.get("Publisher"))
            .or(manifest.get("PackageName"))
            .and_then(|v| v.as_str())
            .is_some();
        let has_default_locale_file = default_locale
            .as_ref()
            .map(|dl| {
                locale_files
                    .iter()
                    .any(|p| p.contains(&format!(".locale.{dl}.yaml")))
            })
            .unwrap_or(false);

        if has_locale_data && !has_default_locale_file {
            let mut entry = manifest.clone();
            if let Some(obj) = entry.as_object_mut()
                && let Some(dl) = &default_locale
            {
                obj.insert("PackageLocale".to_string(), Value::String(dl.clone()));
            }
            main_locale_entry = Some(entry);
        }
    }

    let start = decode_continuation_token(params.continuation_token.as_deref());
    let page_size = WINGET_LOCALES_PAGE_SIZE;
    let main_on_first = main_locale_entry.is_some() && start == 0;
    let locale_start = if main_locale_entry.is_some() && start == 0 {
        0
    } else {
        start.saturating_sub(1)
    };
    let take = page_size.saturating_sub(if main_on_first { 1 } else { 0 });

    let mut locales: Vec<Value> = Vec::new();
    if main_on_first && let Some(e) = main_locale_entry.clone() {
        locales.push(e);
    }

    for locale_path in locale_files.iter().skip(locale_start).take(take) {
        let Some(locale_code) = locale_code_of(locale_path) else {
            continue;
        };
        let Ok(content) = fetch_manifest_content(&storage, locale_path).await else {
            continue;
        };
        let Ok(mut manifest) = parse_yaml(&content) else {
            continue;
        };
        if let Some(obj) = manifest.as_object_mut() {
            obj.insert("PackageLocale".to_string(), Value::String(locale_code));
        }
        locales.push(manifest);
    }

    let total = locale_files.len() + if main_locale_entry.is_some() { 1 } else { 0 };
    let continuation_token =
        (start + page_size < total).then(|| encode_continuation_token(start + page_size));
    json_ok(&LocaleMultipleResponse {
        data: locales,
        continuation_token,
    })
}

/// GET /api/winget/packages/:id/versions/:version/locales/:locale
pub async fn handle_locale(
    State((storage, _db)): State<AppState>,
    Path((package_id, version, locale)): Path<(String, String, String)>,
) -> Response {
    let files = match get_version_manifests(&storage, &package_id, &version).await {
        Ok(f) => f,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if files.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Version {version} of package '{package_id}' not found"),
        );
    }

    let locale_filename = format!("{package_id}.locale.{locale}.yaml");
    let Some(locale_path) = file_by_basename(&files, &locale_filename) else {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Locale '{locale}' not found for version {version} of package '{package_id}'"),
        );
    };

    let content = match fetch_manifest_content(&storage, locale_path).await {
        Ok(c) => c,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut data = match parse_yaml(&content) {
        Ok(m) => m,
        Err(e) => {
            return winget_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to parse locale manifest: {e}"),
            );
        }
    };
    if let Some(obj) = data.as_object_mut() {
        obj.insert("PackageLocale".to_string(), Value::String(locale));
    }
    json_ok(&LocaleSingleResponse { data })
}

/// GET /api/winget/packageManifests/:id
pub async fn handle_package_manifest(
    State((storage, db)): State<AppState>,
    Path(package_id): Path<String>,
    Query(params): Query<ManifestParams>,
) -> Response {
    let versions = match load_versions(&db, &storage, &package_id).await {
        Ok(v) => v,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if versions.is_empty() {
        return winget_error(
            StatusCode::NOT_FOUND,
            &format!("Package '{package_id}' not found"),
        );
    }

    let versions = match params.version.as_ref() {
        Some(fv) if versions.contains(fv) => vec![fv.clone()],
        Some(fv) => {
            return winget_error(
                StatusCode::NOT_FOUND,
                &format!("Version {fv} not found for package '{package_id}'"),
            );
        }
        None => versions,
    };

    let mut manifest_versions: Vec<VersionManifest> = Vec::new();
    for version in &versions {
        let Ok(Some(entry)) = build_version_manifest(&storage, &package_id, version).await else {
            continue;
        };
        // Channel filter.
        if let Some(channel) = &params.channel
            && entry.channel.as_deref() != Some(channel.as_str())
        {
            continue;
        }
        // Market filter (requires at least one matching installer).
        if let Some(market) = &params.market
            && let Some(installers) = &entry.installers
            && !installers.iter().any(|i| market_matches(i, market))
        {
            continue;
        }
        manifest_versions.push(entry);
    }

    json_ok(&PackageManifestResponse {
        data: PackageManifestData {
            package_identifier: package_id,
            versions: manifest_versions,
        },
        unsupported_query_parameters: vec!["FetchAllManifests".to_string()],
        required_query_parameters: vec![],
    })
}

// ── helpers ───────────────────────────────────────────────────────────────

/// Load a package's versions (descending) from index.db.
async fn load_versions(
    db: &SharedDb,
    storage: &SharedStorage,
    package_id: &str,
) -> anyhow::Result<Vec<String>> {
    let conn = get_index_db(db, storage).await?;
    get_package_versions(&conn, package_id)
}

/// Find a manifest path whose final path segment equals `basename`.
fn file_by_basename<'a>(files: &'a [String], basename: &str) -> Option<&'a String> {
    files
        .iter()
        .find(|p| p.rsplit('/').next() == Some(basename))
}

/// Extract the locale code from a locale manifest filename.
fn locale_code_of(path: &str) -> Option<String> {
    let basename = path.rsplit('/').next()?;
    LOCALE_FILE_RE
        .captures(basename)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Whether an installer's Markets block allows the given market (mirrors the TS logic).
fn market_matches(installer: &Value, market: &str) -> bool {
    let Some(markets) = installer.get("Markets") else {
        return true;
    };
    let in_array = |key: &str| {
        markets
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().any(|m| m.as_str() == Some(market)))
            .unwrap_or(false)
    };
    if in_array("AllowedMarkets") {
        return true;
    }
    if in_array("ExcludedMarkets") {
        return false;
    }
    // AllowedMarkets present but the market isn't in it.
    markets.get("AllowedMarkets").is_none()
}
