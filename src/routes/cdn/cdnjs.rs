//! cdnjs CDN route — mirrors cdnjs/[...path].ts.
//!
//! Resolves a library version (exact -> semver range -> latest), serves files from
//! the GitHub raw mirror, and provides directory listings with a trailing-slash
//! listing plus a 404 directory fallback. Supported path forms:
//! - library@version/file
//! - library/version/file
//! - library (resolves latest version + default filename)

use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use node_semver::Version;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::constants::*;
use crate::cdn::listing::{CdnFile, CdnPackageListing};
use crate::cdn::minify::minified_entry;
use crate::cdn::registry::{fetch_cdnjs_files, fetch_cdnjs_library};
use crate::cdn::resolve::max_satisfying;
use crate::cdn::response::file_response;
use crate::cdn::tarball::download_tarball;
use crate::error::AppError;
use crate::storage::{CacheMeta, CdnFileMeta, SharedStorage};

static CDNJS_AT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^@/]+)@([^/]+)(?:/(.*))?$").unwrap());

static VERSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v?\d+\.\d+(\.\d+)?(-[^/]+)?$").unwrap());

/// Shared, read-only context borrowed by the `serve_*` branches.
struct CdnjsCtx<'a> {
    storage: &'a SharedStorage,
    headers: &'a HeaderMap,
    library: &'a str,
    version: &'a str,
    filepath: &'a str,
    cache_base: &'a str,
    cache_control: &'static str,
}

/// Parse `library@version/file` or `library/version/file` (or a bare `library`).
/// "latest" normalizes to an empty version so it resolves to the newest tag. A
/// trailing slash on the file portion is stripped — the root listing is detected
/// separately via the request URI.
fn parse_cdnjs_path(path: &str) -> (String, String, String) {
    let (library, version, mut filepath) = if let Some(caps) = CDNJS_AT_RE.captures(path) {
        let mut v = caps[2].to_string();
        if v == "latest" {
            v.clear();
        }
        let fp = caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string();
        (caps[1].to_string(), v, fp)
    } else {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            let second = parts[1];
            if second == "latest" {
                (parts[0].to_string(), String::new(), parts[2..].join("/"))
            } else if VERSION_RE.is_match(second) {
                (
                    parts[0].to_string(),
                    second.to_string(),
                    parts[2..].join("/"),
                )
            } else {
                (parts[0].to_string(), String::new(), parts[1..].join("/"))
            }
        } else {
            (parts[0].to_string(), String::new(), String::new())
        }
    };

    while filepath.ends_with('/') {
        filepath.pop();
    }

    (library, version, filepath)
}

/// Resolve the version via the cdnjs API (exact -> range -> latest) when unspecified
/// or not a valid semver, and fill the default filename for a bare library root.
/// Returns `(resolved_version, filepath)` with the default filename applied.
async fn resolve_cdnjs_version(
    storage: &SharedStorage,
    library: &str,
    mut version: String,
    mut filepath: String,
) -> Result<(String, String), AppError> {
    if version.is_empty() || Version::parse(&version).is_err() {
        let data = fetch_cdnjs_library(storage, library)
            .await
            .map_err(|_| AppError::not_found("Library not found"))?;

        let all_versions: Vec<String> = data["versions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if !all_versions.is_empty() {
            // Exact match first.
            let mut found = all_versions.iter().any(|v| v == &version);

            // Range match (e.g. "3", "3.7", "^3.7.0") — mirrors semver.maxSatisfying.
            if !found
                && !version.is_empty()
                && let Some(matched) = max_satisfying(&all_versions, &version)
            {
                version = matched;
                found = true;
            }

            // Fallback to the newest version — mirrors [...all].sort(rcompare)[0].
            if !found {
                version = max_satisfying(&all_versions, "*")
                    .unwrap_or_else(|| data["version"].as_str().unwrap_or("").to_string());
            }
        } else {
            version = data["version"].as_str().unwrap_or("").to_string();
        }

        // Default filename for a bare library root.
        if filepath.is_empty() {
            filepath = data["filename"].as_str().unwrap_or("").to_string();
        }
    }

    Ok((version, filepath))
}

pub async fn handle_cdnjs(
    State((storage, _)): State<(SharedStorage, crate::winget::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    if path.is_empty() {
        return Err(AppError::bad_request("Invalid path"));
    }
    let has_trailing_slash = uri.to_string().ends_with('/');

    let (library, version, filepath) = parse_cdnjs_path(&path);
    let (version, filepath) = resolve_cdnjs_version(&storage, &library, version, filepath).await?;

    let cacheable = Version::parse(&version).is_ok();
    let cache_control = if cacheable {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };
    let cache_base = format!("cdn/cdnjs/{library}/{version}");

    let ctx = CdnjsCtx {
        storage: &storage,
        headers: &headers,
        library: &library,
        version: &version,
        filepath: &filepath,
        cache_base: &cache_base,
        cache_control,
    };

    if filepath.is_empty() {
        if has_trailing_slash {
            serve_cdnjs_root_listing(&ctx).await
        } else {
            serve_cdnjs_root_file(&ctx).await
        }
    } else {
        serve_cdnjs_subpath(&ctx).await
    }
}

/// Library root without a trailing slash: serve the default file from the API.
async fn serve_cdnjs_root_file(ctx: &CdnjsCtx<'_>) -> Result<Response, AppError> {
    let filename = fetch_cdnjs_library(ctx.storage, ctx.library)
        .await
        .map_err(|_| AppError::not_found("Library not found"))?["filename"]
        .as_str()
        .ok_or_else(|| AppError::not_found("No default filename"))?
        .to_string();
    let original =
        get_cdnjs_file(ctx.storage, ctx.library, ctx.version, &filename, ctx.cache_base).await?;
    // jsDelivr: the default file is always minified (see npm route).
    let file_data = minified_entry(ctx.storage, ctx.cache_base, &filename, &original).await;
    Ok(file_response(
        &filename,
        &file_data,
        ctx.cache_control,
        ctx.headers,
        None,
    ))
}

/// Library root with a trailing slash: list all files for the resolved version.
async fn serve_cdnjs_root_listing(ctx: &CdnjsCtx<'_>) -> Result<Response, AppError> {
    let files = ensure_cdnjs_file_list_cached(ctx.storage, ctx.library, ctx.version, ctx.cache_base)
        .await
        .map_err(|_| AppError::not_found("Version not found"))?;
    let listing = CdnPackageListing {
        name: Some(ctx.library.to_string()),
        version: Some(ctx.version.to_string()),
        path: String::new(),
        files: files
            .into_iter()
            .map(|name| CdnFile {
                name,
                size: 0,
                integrity: None,
            })
            .collect(),
    };
    let body = serde_json::to_string(&listing)?;
    Ok(json_listing(body))
}

/// Sub-path file with a directory-listing fallback on 404.
async fn serve_cdnjs_subpath(ctx: &CdnjsCtx<'_>) -> Result<Response, AppError> {
    match get_cdnjs_file(
        ctx.storage,
        ctx.library,
        ctx.version,
        ctx.filepath,
        ctx.cache_base,
    )
    .await
    {
        Ok(file_data) => Ok(file_response(
            ctx.filepath,
            &file_data,
            ctx.cache_control,
            ctx.headers,
            None,
        )),
        Err(_) => {
            let files = get_cached_file_list(ctx.storage, ctx.cache_base).await;
            let prefix = format!("{}/", ctx.filepath);
            let mut dir: Vec<CdnFile> = files
                .into_iter()
                .filter(|f| f.starts_with(&prefix))
                .map(|f| CdnFile {
                    name: f[prefix.len()..].to_string(),
                    size: 0,
                    integrity: None,
                })
                .filter(|f| !f.name.is_empty())
                .collect();
            if dir.is_empty() {
                return Err(AppError::not_found(format!(
                    "Path not found: {}",
                    ctx.filepath
                )));
            }
            dir.sort_by(|a, b| a.name.cmp(&b.name));
            let listing = CdnPackageListing {
                name: Some(ctx.library.to_string()),
                version: Some(ctx.version.to_string()),
                path: ctx.filepath.to_string(),
                files: dir,
            };
            let body = serde_json::to_string(&listing)?;
            Ok(json_listing(body))
        }
    }
}

/// Build a JSON directory-listing response.
fn json_listing(body: String) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", CDN_CACHE_SHORT),
            ("vary", "Accept-Encoding"),
        ],
        body,
    )
        .into_response()
}

/// Fetch a cdnjs file from cache or the GitHub raw mirror, and background-warm the
/// version file list so a subsequent directory listing is served from cache.
async fn get_cdnjs_file(
    storage: &SharedStorage,
    library: &str,
    version: &str,
    filepath: &str,
    cache_base: &str,
) -> Result<Vec<u8>, AppError> {
    let cache_key = format!("{cache_base}/{filepath}");

    if let Some(cached) = storage.get_raw(&cache_key).await {
        return Ok(cached);
    }

    let url = format!(
        "https://raw.githubusercontent.com/cdnjs/cdnjs/refs/heads/master/ajax/libs/{library}/{version}/{filepath}"
    );
    let data = download_tarball(&url)
        .await
        .map_err(|_| AppError::not_found("File not found"))?;

    storage.set_raw(&cache_key, &data).await;

    // Background-warm the version file list for directory listings.
    let s = storage.clone();
    let lib = library.to_string();
    let ver = version.to_string();
    let base = cache_base.to_string();
    tokio::spawn(async move {
        let _ = ensure_cdnjs_file_list_cached(&s, &lib, &ver, &base).await;
    });

    Ok(data)
}

/// Ensure the version file list is cached, returning it from cache or the cdnjs API.
async fn ensure_cdnjs_file_list_cached(
    storage: &SharedStorage,
    library: &str,
    version: &str,
    cache_base: &str,
) -> anyhow::Result<Vec<String>> {
    if let Some(meta) = storage.get_meta(cache_base).await
        && let Some(files) = meta.files
    {
        return Ok(files.into_iter().map(|f| f.name).collect());
    }

    let data = fetch_cdnjs_files(library, version).await?;
    let files: Vec<String> = data["files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    storage
        .set_meta(
            cache_base,
            &CacheMeta {
                files: Some(
                    files
                        .iter()
                        .map(|name| CdnFileMeta {
                            name: name.clone(),
                            size: 0,
                            integrity: None,
                        })
                        .collect(),
                ),
                ..Default::default()
            },
        )
        .await;

    Ok(files)
}

/// Read the cached version file list (empty if not yet cached).
async fn get_cached_file_list(storage: &SharedStorage, cache_base: &str) -> Vec<String> {
    storage
        .get_meta(cache_base)
        .await
        .and_then(|m| m.files)
        .map(|files| files.into_iter().map(|f| f.name).collect())
        .unwrap_or_default()
}
