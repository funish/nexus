//! `/cdn/combine/{u1},{u2},...` — concatenate multiple npm/gh files into one
//! response (jsDelivr-compatible).
//!
//! Paths are comma-separated and prefixed with `npm/` or `gh/`, e.g.
//! `npm/jquery@3/dist/jquery.min.js,gh/twbs/bootstrap@5/dist/js/bootstrap.min.js`.
//! Each part is resolved and fetched via the same registry/tarball path as the
//! single-file routes; the results are joined with newlines and cached long-term.

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use node_semver::Version;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::mime::get_content_type;
use crate::cdn::utils::registry::{fetch_github_tags, fetch_npm_metadata};
use crate::cdn::utils::resolve::{resolve_from_tags, resolve_registry_version};
use crate::cdn::utils::tarball::extract_file_from_tarball;
use crate::error::AppError;
use crate::storage::SharedStorage;

static NPM_PART_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^npm/(@[^/]+/[^@/]+|[^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

static GH_PART_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^gh/([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

pub async fn handle_combine(
    State((storage, _)): State<(SharedStorage, crate::winget::utils::db::SharedDb)>,
    Path(paths): Path<String>,
) -> Result<Response, AppError> {
    let parts: Vec<&str> = paths
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 {
        return Err(AppError::bad_request("Combine requires at least 2 URLs"));
    }

    let mut combined: Vec<u8> = Vec::new();
    let mut first_type: Option<String> = None;
    // Immutable only when every part names an exact version; any latest/range alias can
    // move, so the combined result must be revalidated (jsDelivr rule).
    let mut all_exact = true;

    for part in &parts {
        match fetch_part(&storage, part).await? {
            Some((data, content_type, is_exact)) => {
                if first_type.is_none() {
                    first_type = Some(content_type);
                }
                if !is_exact {
                    all_exact = false;
                }
                combined.extend_from_slice(&data);
                combined.push(b'\n');
            }
            None => {
                return Err(AppError::not_found(format!(
                    "File not found in combine: {part}"
                )));
            }
        }
    }

    let content_type =
        first_type.unwrap_or_else(|| "application/javascript; charset=utf-8".to_string());
    let cache_control = if all_exact {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };
    let mut resp = (
        StatusCode::OK,
        [
            ("cache-control", cache_control),
            ("vary", "Accept-Encoding"),
        ],
        combined,
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&content_type) {
        resp.headers_mut().insert("content-type", v);
    }
    Ok(resp)
}

/// Resolve and fetch a single combine part. Returns `(data, content_type)` on hit.
async fn fetch_part(
    storage: &SharedStorage,
    part: &str,
) -> Result<Option<(Vec<u8>, String, bool)>, AppError> {
    if let Some(caps) = NPM_PART_RE.captures(part) {
        let package = &caps[1];
        let version = caps.get(2).map(|m| m.as_str()).unwrap_or("latest");
        let filepath = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        if filepath.is_empty() {
            return Ok(None);
        }
        let metadata = fetch_npm_metadata(storage, package)
            .await
            .map_err(|_| AppError::bad_gateway("npm metadata fetch failed"))?;
        let Some(resolved) = resolve_registry_version(&metadata, version) else {
            return Ok(None);
        };
        let Some(tarball) = resolved.version_info["dist"]["tarball"].as_str() else {
            return Ok(None);
        };
        let cache_base = format!("cdn/npm/{package}/{}", resolved.version);
        let data = extract_file_from_tarball(
            storage,
            tarball,
            filepath,
            &format!("{cache_base}/{filepath}"),
            None,
            None,
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;
        return Ok(Some((
            data,
            get_content_type(filepath),
            version == resolved.version,
        )));
    }

    if let Some(caps) = GH_PART_RE.captures(part) {
        let owner = &caps[1];
        let repo = &caps[2];
        let version = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        let filepath = caps.get(4).map(|m| m.as_str()).unwrap_or("");
        if filepath.is_empty() {
            return Ok(None);
        }
        let tags = fetch_github_tags(storage, owner, repo)
            .await
            .unwrap_or_default();
        let resolved = if version.is_empty() {
            resolve_from_tags(&tags, "*").unwrap_or_else(|| "main".to_string())
        } else {
            resolve_from_tags(&tags, version).unwrap_or_else(|| version.to_string())
        };
        let tarball_url =
            format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/tags/{resolved}");
        let cache_base = format!("cdn/gh/{owner}/{repo}/{resolved}");
        let raw_url =
            format!("https://raw.githubusercontent.com/{owner}/{repo}/{resolved}/{filepath}");
        let data = extract_file_from_tarball(
            storage,
            &tarball_url,
            filepath,
            &format!("{cache_base}/{filepath}"),
            Some(&raw_url),
            None,
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;
        return Ok(Some((
            data,
            get_content_type(filepath),
            Version::parse(version).is_ok(),
        )));
    }

    Ok(None)
}
