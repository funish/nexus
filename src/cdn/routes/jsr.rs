//! JSR registry CDN route — mirrors jsr/[...path].ts.
//!
//! Resolves a JSR package version, background-caches the tarball, and serves the
//! entry file (or sub-path) with ETag/304 support and a directory-listing fallback.

use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use node_semver::Version;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::listing::{CdnPackageListing, get_directory_listing};
use crate::cdn::utils::mime::get_content_type;
use crate::cdn::utils::registry::fetch_jsr_metadata;
use crate::cdn::utils::resolve::resolve_registry_version;
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
    let cacheable = Version::parse(&resolved.version).is_ok();
    let is_cached = is_package_cached(&storage, &cache_base, cacheable).await;

    let entry_file = resolve_jsr_entry(&metadata);
    let resolved_version = resolved.version.as_str();
    let cache_control = if cacheable {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };

    // Background-cache the entire package when not yet cached.
    if !is_cached {
        let s = storage.clone();
        let u = tarball_url.clone();
        let b = cache_base.clone();
        let l = format!("jsr:{package_name}@{resolved_version}");
        tokio::spawn(async move {
            let _ = cache_package_from_tarball(&s, &u, &b, &l).await;
        });
    }

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
            return Ok(json_listing(body));
        }

        let file_data = extract_file_from_tarball(
            &storage,
            &tarball_url,
            &entry_file,
            &format!("{cache_base}/{entry_file}"),
            None,
        )
        .await
        .map_err(|_| AppError::not_found(format!("Entry file not found: {entry_file}")))?;

        let integrity = file_integrity(&storage, &cache_base, &entry_file).await;
        return Ok(serve_file(
            file_data,
            &entry_file,
            cache_control,
            integrity,
            resolved_version,
            &headers,
        ));
    }

    // Sub-path file with directory-listing fallback on 404.
    match extract_file_from_tarball(
        &storage,
        &tarball_url,
        filepath,
        &format!("{cache_base}/{filepath}"),
        None,
    )
    .await
    {
        Ok(file_data) => {
            let integrity = file_integrity(&storage, &cache_base, filepath).await;
            Ok(serve_file(
                file_data,
                filepath,
                cache_control,
                integrity,
                resolved_version,
                &headers,
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
                    Ok(json_listing(body))
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

/// Look up a cached file's integrity (SRI) for use as an ETag.
async fn file_integrity(storage: &SharedStorage, cache_base: &str, file: &str) -> Option<String> {
    let meta = storage.get_meta(cache_base).await?;
    let files = meta.files?;
    files
        .iter()
        .find(|f| f.name == file)
        .and_then(|f| f.integrity.clone())
}

/// Serve a file body with ETag/304 handling, content-type, cache-control, and X-Resolved-Version.
fn serve_file(
    file_data: Vec<u8>,
    filename: &str,
    cache_control: &'static str,
    integrity: Option<String>,
    resolved_version: &str,
    headers: &HeaderMap,
) -> Response {
    // 304 short-circuit when the client already holds this exact content.
    if let Some(ref etag) = integrity {
        let matches = headers
            .get("if-none-match")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == etag.as_str())
            .unwrap_or(false);
        if matches {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            if let Ok(v) = HeaderValue::from_str(etag) {
                resp.headers_mut().insert("etag", v);
            }
            return resp;
        }
    }

    let mut resp = (
        StatusCode::OK,
        [
            ("cache-control", cache_control),
            ("vary", "Accept-Encoding"),
        ],
        file_data,
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&get_content_type(filename)) {
        resp.headers_mut().insert("content-type", v);
    }
    if let Some(etag) = integrity
        && let Ok(v) = HeaderValue::from_str(&etag)
    {
        resp.headers_mut().insert("etag", v);
    }
    if let Ok(v) = HeaderValue::from_str(resolved_version) {
        resp.headers_mut().insert("x-resolved-version", v);
    }
    resp
}

/// Build a JSON directory-listing response.
fn json_listing(body: String) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", CDN_CACHE_LONG),
            ("vary", "Accept-Encoding"),
        ],
        body,
    )
        .into_response()
}
