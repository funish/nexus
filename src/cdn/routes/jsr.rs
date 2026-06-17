//! JSR registry CDN route — mirrors jsr/[...path].ts.
//!
//! Resolves a JSR package version, background-caches the tarball, and serves the
//! entry file (or sub-path) with ETag/304 support and a directory-listing fallback.

use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::listing::{CdnPackageListing, get_directory_listing};
use crate::cdn::utils::minify::minified_entry;
use crate::cdn::utils::registry::fetch_jsr_metadata;
use crate::cdn::utils::resolve::resolve_registry_version;
use crate::cdn::utils::response::file_response_versioned;
use crate::cdn::utils::tarball::{
    cache_package_from_tarball, extract_file_from_tarball, is_package_cached,
};
use crate::error::AppError;
use crate::storage::SharedStorage;

static JSR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

pub async fn handle_jsr(
    State((storage, _)): State<(SharedStorage, crate::winget::utils::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let has_trailing_slash = uri.to_string().ends_with('/');

    let caps = JSR_RE
        .captures(&path)
        .ok_or_else(|| AppError::bad_request("Invalid JSR package path format"))?;

    let scope = &caps[1];
    let package = &caps[2];
    let version = caps.get(3).map(|m| m.as_str()).unwrap_or("latest");
    let filepath = caps.get(4).map(|m| m.as_str()).unwrap_or("");

    let metadata = fetch_jsr_metadata(&storage, scope, package)
        .await
        .map_err(|_| AppError::not_found("JSR package not found"))?;

    let resolved = resolve_registry_version(&metadata, version)
        .ok_or_else(|| AppError::not_found("Version not found"))?;

    let package_name = format!("@{scope}/{package}");
    let tarball_url = resolved.version_info["dist"]["tarball"]
        .as_str()
        .ok_or_else(|| AppError::bad_gateway("Missing tarball URL"))?
        .to_string();

    let cache_base = format!("cdn/jsr/{package_name}/{}", resolved.version);
    // Immutable only for an exact-version request; latest/range aliases resolve to an
    // exact version but can move, so they use the short tag TTL (jsDelivr rule).
    let cacheable = resolved.version == version;
    let cached_meta = is_package_cached(&storage, &cache_base, cacheable).await;
    let is_cached = cached_meta.is_some();

    let entry_file = resolve_jsr_entry(&metadata);
    let resolved_version = resolved.version.as_str();
    let cache_control = if cacheable {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };

    // The foreground entry/sub-path request downloads the tarball; extract_file_from_tarball
    // warms the full package in the background reusing those bytes (no second download),
    // so no upfront background spawn is needed. Directory-listing requests still cache
    // the whole package up front below.
    let warm_label = format!("jsr:{package_name}@{resolved_version}");
    let warm = (!is_cached).then_some((cache_base.as_str(), warm_label.as_str()));

    // Package root: trailing slash -> directory listing; otherwise entry file.
    if filepath.is_empty() {
        if has_trailing_slash {
            if !is_cached {
                cache_package_from_tarball(
                    &storage,
                    &tarball_url,
                    &cache_base,
                    &format!("jsr:{package_name}@{resolved_version}"),
                )
                .await
                .map_err(|e| AppError::bad_gateway(e.to_string()))?;
            }
            let listing =
                get_directory_listing(&storage, &cache_base, "", &package_name, resolved_version)
                    .await
                    .unwrap_or(CdnPackageListing {
                        name: Some(package_name),
                        version: Some(resolved_version.to_string()),
                        path: String::new(),
                        files: vec![],
                    });
            let body = serde_json::to_string(&listing)?;
            return Ok(json_listing(body, cache_control));
        }

        let original = extract_file_from_tarball(
            &storage,
            &tarball_url,
            &entry_file,
            &format!("{cache_base}/{entry_file}"),
            None,
            warm,
        )
        .await
        .map_err(|_| AppError::not_found(format!("Entry file not found: {entry_file}")))?;

        // jsDelivr: the default file is always minified (see npm route).
        let file_data = minified_entry(&storage, &cache_base, &entry_file, &original).await;
        return Ok(file_response_versioned(
            &entry_file,
            &file_data,
            cache_control,
            &headers,
            resolved_version,
            None,
        ));
    }

    // Sub-path file with directory-listing fallback on 404.
    match extract_file_from_tarball(
        &storage,
        &tarball_url,
        filepath,
        &format!("{cache_base}/{filepath}"),
        None,
        warm,
    )
    .await
    {
        Ok(file_data) => {
            // Reuse the cached per-file integrity as the ETag (the meta was
            // already loaded by is_package_cached) instead of re-hashing.
            let etag = cached_meta
                .as_ref()
                .and_then(|m| m.files.as_ref())
                .and_then(|files| files.iter().find(|f| f.name == filepath))
                .and_then(|f| f.integrity.as_deref());
            Ok(file_response_versioned(
                filepath,
                &file_data,
                cache_control,
                &headers,
                resolved_version,
                etag,
            ))
        }
        Err(_) => {
            if !is_cached {
                return Err(AppError::not_found(format!(
                    "Path not found: {filepath}. Package not yet cached."
                )));
            }
            match get_directory_listing(
                &storage,
                &cache_base,
                filepath,
                &package_name,
                resolved_version,
            )
            .await
            {
                Some(listing) => {
                    let body = serde_json::to_string(&listing)?;
                    Ok(json_listing(body, cache_control))
                }
                None => Err(AppError::not_found(format!("Path not found: {filepath}"))),
            }
        }
    }
}

/// Resolve the JSR entry file from registry metadata exports.
/// Mirrors jsr/[...path].ts package.json exports handling (string / ".".default / ".".import).
fn resolve_jsr_entry(metadata: &serde_json::Value) -> String {
    let exports = &metadata["exports"];
    if let Some(s) = exports.as_str() {
        return s.strip_prefix("./").unwrap_or(s).to_string();
    }
    if let Some(dot) = exports.get(".") {
        if let Some(s) = dot.as_str() {
            return s.strip_prefix("./").unwrap_or(s).to_string();
        }
        for cond in ["default", "import"] {
            if let Some(s) = dot[cond].as_str() {
                return s.strip_prefix("./").unwrap_or(s).to_string();
            }
        }
    }
    "mod.ts".to_string()
}

/// Build a JSON directory-listing response.
fn json_listing(body: String, cache_control: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", cache_control),
            ("vary", "Accept-Encoding"),
        ],
        body,
    )
        .into_response()
}
