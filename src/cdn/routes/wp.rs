//! WordPress plugins/themes CDN route.
//!
//! Mirrors wp/[...path].ts: fetches files from WordPress SVN with a jsDelivr
//! fallback. Trunk refs use the branch-tier cache (12h); tagged versions are immutable (1y).
//!
//! Supported paths:
//! - /cdn/wp/plugins/<name>/tags/<version>/<file>
//! - /cdn/wp/plugins/<name>/trunk/<file>
//! - /cdn/wp/themes/<name>/<version>/<file>

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::{CDN_CACHE_BRANCH, CDN_CACHE_LONG};
use crate::cdn::utils::tarball::try_fetch;
use crate::error::AppError;
use crate::storage::SharedStorage;

static THEME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^themes/([^/]+)/([^/]+)/(.*)$").unwrap());
static PLUGIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^plugins/([^/]+)/(tags|trunk)(?:/([^/]+))?(?:/(.*))?$").unwrap());

pub async fn handle_wp(
    State((storage, _)): State<(SharedStorage, crate::winget::utils::db::SharedDb)>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    if path.is_empty() {
        return Err(AppError::bad_request("Invalid path"));
    }

    let (svn_url, jsdelivr_url, cache_key, is_trunk) = if path.starts_with("themes/") {
        let caps = THEME_RE
            .captures(&path)
            .ok_or_else(|| AppError::bad_request("Invalid WordPress theme path format"))?;
        let theme = &caps[1];
        let version = &caps[2];
        let file = &caps[3];
        (
            format!("https://themes.svn.wordpress.org/{theme}/{version}/{file}"),
            format!("https://cdn.jsdelivr.net/wp/themes/{theme}/{version}/{file}"),
            format!("cdn/wp/themes/{theme}/{version}/{file}"),
            false,
        )
    } else {
        let caps = PLUGIN_RE
            .captures(&path)
            .ok_or_else(|| AppError::bad_request("Invalid WordPress plugin path format"))?;
        let plugin = &caps[1];
        let reference = &caps[2];
        let version = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        let file = caps.get(4).map(|m| m.as_str()).unwrap_or("");
        let is_trunk = reference == "trunk";

        let (svn_url, jsdelivr_url, cache_key) = if reference == "trunk" {
            (
                format!("https://plugins.svn.wordpress.org/{plugin}/trunk/{file}"),
                format!("https://cdn.jsdelivr.net/wp/{plugin}/trunk/{file}"),
                format!("cdn/wp/plugins/{plugin}/trunk/{file}"),
            )
        } else {
            if version.is_empty() || file.is_empty() {
                return Err(AppError::bad_request(
                    "Invalid WordPress plugin path format. Use /wp/plugins/plugin-name/tags/version/file",
                ));
            }
            (
                format!("https://plugins.svn.wordpress.org/{plugin}/tags/{version}/{file}"),
                format!("https://cdn.jsdelivr.net/wp/{plugin}/tags/{version}/{file}"),
                format!("cdn/wp/plugins/{plugin}/tags/{version}/{file}"),
            )
        };
        (svn_url, jsdelivr_url, cache_key, is_trunk)
    };

    // Check cache first.
    if let Some(cached) = storage.get_raw(&cache_key).await {
        return Ok(file_response(&svn_url, &cached, is_trunk, &headers));
    }

    // Try WordPress SVN, then jsDelivr fallback.
    let data = match try_fetch(&svn_url).await {
        Some(d) => d,
        None => match try_fetch(&jsdelivr_url).await {
            Some(d) => d,
            None => return Err(AppError::not_found("Resource not found")),
        },
    };

    // Cache in the background without blocking the response.
    let s = storage.clone();
    let k = cache_key.clone();
    let data_for_cache = data.clone();
    tokio::spawn(async move {
        s.set_raw(&k, &data_for_cache).await;
    });

    Ok(file_response(&svn_url, &data, is_trunk, &headers))
}

/// Build a file response, deriving filename/cache-control from the SVN URL and ref kind,
/// then delegating to the shared ETag/304 builder.
fn file_response(svn_url: &str, data: &[u8], is_trunk: bool, headers: &HeaderMap) -> Response {
    let filename = svn_url.rsplit('/').next().unwrap_or("");
    let cache_control = if is_trunk {
        CDN_CACHE_BRANCH
    } else {
        CDN_CACHE_LONG
    };
    crate::cdn::utils::response::file_response(filename, data, cache_control, headers, None)
}
