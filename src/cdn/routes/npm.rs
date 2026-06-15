use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use node_semver::Version;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::entry::{
    ENTRY_FALLBACKS, resolve_default_file, resolve_esm_entry, resolve_style_file,
};
use crate::cdn::utils::esm::{EsmBundleOptions, bundle_esm_package};
use crate::cdn::utils::listing::{CdnOrgListing, CdnPackageListing, get_directory_listing};
use crate::cdn::utils::mime::get_content_type;
use crate::cdn::utils::minify::minify_for;
use crate::cdn::utils::registry::fetch_npm_metadata;
use crate::cdn::utils::resolve::resolve_registry_version;
use crate::cdn::utils::tarball::{
    cache_package_from_tarball, extract_file_from_tarball, is_package_cached,
};
use crate::error::AppError;
use crate::storage::SharedStorage;

static SCOPED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

static NORMAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

static ORG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@([^/]+)/?$").unwrap());

pub async fn handle_npm(
    State((storage, _)): State<(SharedStorage, crate::winget::utils::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let original_url = uri.to_string();
    let has_trailing_slash = original_url.ends_with('/');

    // Parse path
    let (package_name, version, filepath) = if path.starts_with('@') {
        // Check for org listing: @scope or @scope/
        if let Some(caps) = ORG_RE.captures(&path) {
            let scope = &caps[1];
            let packages = crate::cdn::utils::registry::fetch_org_packages(&storage, scope)
                .await
                .map_err(|_| AppError::not_found("Organization not found"))?;

            let body = serde_json::to_string(&CdnOrgListing {
                name: format!("@{scope}"),
                packages,
            })?;

            return Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    ("cache-control", CDN_CACHE_SHORT),
                ],
                body,
            )
                .into_response());
        }

        let caps = SCOPED_RE
            .captures(&path)
            .ok_or_else(|| AppError::bad_request("Invalid scoped package path format"))?;
        let scope = &caps[1];
        let pkg = &caps[2];
        let ver = caps.get(3).map(|m| m.as_str()).unwrap_or("latest");
        let fp = caps.get(4).map(|m| m.as_str()).unwrap_or("");
        (format!("@{scope}/{pkg}"), ver.to_string(), fp.to_string())
    } else {
        let caps = NORMAL_RE
            .captures(&path)
            .ok_or_else(|| AppError::bad_request("Invalid package path format"))?;
        let pkg = &caps[1];
        let ver = caps.get(2).map(|m| m.as_str()).unwrap_or("latest");
        let fp = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        (pkg.to_string(), ver.to_string(), fp.to_string())
    };

    // Fetch metadata
    let metadata = fetch_npm_metadata(&storage, &package_name)
        .await
        .map_err(|_| AppError::not_found("Package not found"))?;

    // Resolve version
    let resolved = resolve_registry_version(&metadata, &version)
        .ok_or_else(|| AppError::not_found("Version not found"))?;

    let tarball_url = resolved.version_info["dist"]["tarball"]
        .as_str()
        .ok_or_else(|| AppError::bad_gateway("Missing tarball URL"))?
        .to_string();

    let cache_base = format!("cdn/npm/{package_name}/{}", resolved.version);
    let cacheable = Version::parse(&resolved.version).is_ok();
    let is_cached = is_package_cached(&storage, &cache_base, cacheable).await;

    // +esm bundling
    if filepath == "+esm" {
        let entry_file =
            resolve_esm_entry(&resolved.version_info).unwrap_or_else(|| "index.js".to_string());

        if !is_cached {
            cache_package_from_tarball(
                &storage,
                &tarball_url,
                &cache_base,
                &format!("npm:{package_name}@{}", resolved.version),
            )
            .await
            .map_err(|e| AppError::bad_gateway(e.to_string()))?;
        }

        let code = bundle_esm_package(
            &storage,
            &EsmBundleOptions {
                package_name: package_name.clone(),
                version: resolved.version.clone(),
                entry_point: entry_file,
            },
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;

        return Ok((
            StatusCode::OK,
            [
                ("content-type", "application/javascript; charset=utf-8"),
                ("cache-control", CDN_CACHE_LONG),
                ("vary", "Accept-Encoding"),
                ("x-resolved-version", &resolved.version),
            ],
            code,
        )
            .into_response());
    }

    // Root path
    if filepath.is_empty() {
        if has_trailing_slash {
            // Directory listing
            if !is_cached {
                cache_package_from_tarball(
                    &storage,
                    &tarball_url,
                    &cache_base,
                    &format!("npm:{package_name}@{}", resolved.version),
                )
                .await
                .map_err(|e| AppError::bad_gateway(e.to_string()))?;
            }

            let listing =
                get_directory_listing(&storage, &cache_base, "", &package_name, &resolved.version)
                    .await;

            let body = serde_json::to_string(&listing.unwrap_or(CdnPackageListing {
                name: Some(package_name),
                version: Some(resolved.version.clone()),
                path: String::new(),
                files: vec![],
            }))?;

            return Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    ("cache-control", CDN_CACHE_LONG),
                    ("vary", "Accept-Encoding"),
                    ("x-resolved-version", &resolved.version),
                ],
                body,
            )
                .into_response());
        } else {
            // Entry file — jsDelivr priority (jsdelivr > browser > main, then CSS `style`),
            // then common fallback filenames tried against the actual package contents.
            let entry_candidates = resolve_default_file(&resolved.version_info)
                .or_else(|| resolve_style_file(&resolved.version_info))
                .into_iter()
                .chain(ENTRY_FALLBACKS.iter().map(|s| (*s).to_string()));

            let (entry_file, file_data) = {
                let mut found = None;
                for cand in entry_candidates {
                    if let Ok(data) = extract_file_from_tarball(
                        &storage,
                        &tarball_url,
                        &cand,
                        &format!("{cache_base}/{cand}"),
                        None,
                    )
                    .await
                    {
                        found = Some((cand, data));
                        break;
                    }
                }
                found.ok_or_else(|| AppError::not_found("Entry file not found"))?
            };

            if !is_cached {
                let storage_clone = storage.clone();
                let url = tarball_url.clone();
                let base = cache_base.clone();
                let label = format!("npm:{package_name}@{}", resolved.version);
                tokio::spawn(async move {
                    let _ = cache_package_from_tarball(&storage_clone, &url, &base, &label).await;
                });
            }

            // jsDelivr: the default file is always minified.
            let file_data = minify_for(&entry_file, &file_data);
            let etag = crate::cdn::utils::integrity::calculate_integrity(&file_data);

            if headers
                .get("if-none-match")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == etag)
            {
                return Ok(StatusCode::NOT_MODIFIED.into_response());
            }

            let cache_control = if Version::parse(&resolved.version).is_ok() {
                CDN_CACHE_LONG
            } else {
                CDN_CACHE_TAG
            };

            let mut resp = (
                StatusCode::OK,
                [
                    ("cache-control", cache_control),
                    ("etag", etag.as_str()),
                    ("vary", "Accept-Encoding"),
                    ("x-resolved-version", resolved.version.as_str()),
                ],
                file_data,
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&get_content_type(&entry_file)) {
                resp.headers_mut().insert("content-type", v);
            }
            return Ok(resp);
        }
    }

    // Sub-path file
    match extract_file_from_tarball(
        &storage,
        &tarball_url,
        &filepath,
        &format!("{cache_base}/{filepath}"),
        None,
    )
    .await
    {
        Ok(file_data) => {
            if !is_cached {
                let storage_clone = storage.clone();
                let url = tarball_url.clone();
                let base = cache_base.clone();
                let label = format!("npm:{package_name}@{}", resolved.version);
                tokio::spawn(async move {
                    let _ = cache_package_from_tarball(&storage_clone, &url, &base, &label).await;
                });
            }

            // ETag check
            if let Some(meta) = storage.get_meta(&cache_base).await
                && let Some(files) = &meta.files
                && let Some(f) = files.iter().find(|f| f.name == filepath)
                && let Some(integrity) = &f.integrity
                && let Some(if_none_match) =
                    headers.get("if-none-match").and_then(|v| v.to_str().ok())
                && if_none_match == integrity
            {
                return Ok(StatusCode::NOT_MODIFIED.into_response());
            }

            let cache_control = if Version::parse(&resolved.version).is_ok() {
                CDN_CACHE_LONG
            } else {
                CDN_CACHE_TAG
            };

            let mut resp = (
                StatusCode::OK,
                [
                    ("cache-control", cache_control),
                    ("vary", "Accept-Encoding"),
                    ("x-resolved-version", resolved.version.as_str()),
                ],
                file_data,
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&get_content_type(&filepath)) {
                resp.headers_mut().insert("content-type", v);
            }
            Ok(resp)
        }
        Err(_) => {
            // jsDelivr `.min` synthesis: foo.min.js requested but only foo.js exists.
            if is_cached
                && let Some(orig) = crate::cdn::utils::minify::strip_min_suffix(&filepath)
                && let Ok(orig_data) = extract_file_from_tarball(
                    &storage,
                    &tarball_url,
                    &orig,
                    &format!("{cache_base}/{orig}"),
                    None,
                )
                .await
            {
                let minified = minify_for(&orig, &orig_data);
                // Cache the synthesized .min file so subsequent requests hit storage directly.
                let s = storage.clone();
                let (k, d) = (format!("{cache_base}/{filepath}"), minified.clone());
                tokio::spawn(async move {
                    s.set_raw(&k, &d).await;
                });

                let etag = crate::cdn::utils::integrity::calculate_integrity(&minified);
                let cache_control = if Version::parse(&resolved.version).is_ok() {
                    CDN_CACHE_LONG
                } else {
                    CDN_CACHE_TAG
                };
                let mut resp = (
                    StatusCode::OK,
                    [
                        ("cache-control", cache_control),
                        ("etag", etag.as_str()),
                        ("vary", "Accept-Encoding"),
                        ("x-resolved-version", resolved.version.as_str()),
                    ],
                    minified,
                )
                    .into_response();
                if let Ok(v) = HeaderValue::from_str(&get_content_type(&filepath)) {
                    resp.headers_mut().insert("content-type", v);
                }
                return Ok(resp);
            }

            // Version fallback (jsDelivr): the newest version matching the range lacks
            // this file — try the next matching versions before giving up.
            if version != resolved.version {
                let candidates =
                    crate::cdn::utils::resolve::resolve_registry_versions_desc(&metadata, &version);
                for cand in candidates.into_iter().skip(1).take(2) {
                    if let Some(info) = metadata["versions"].get(cand.as_str())
                        && let Some(tb) = info["dist"]["tarball"].as_str()
                    {
                        let cand_base = format!("cdn/npm/{package_name}/{cand}");
                        if let Ok(data) = extract_file_from_tarball(
                            &storage,
                            tb,
                            &filepath,
                            &format!("{cand_base}/{filepath}"),
                            None,
                        )
                        .await
                        {
                            let cache_control = if Version::parse(&cand).is_ok() {
                                CDN_CACHE_LONG
                            } else {
                                CDN_CACHE_TAG
                            };
                            let mut resp = (
                                StatusCode::OK,
                                [
                                    ("cache-control", cache_control),
                                    ("vary", "Accept-Encoding"),
                                    ("x-resolved-version", cand.as_str()),
                                ],
                                data,
                            )
                                .into_response();
                            if let Ok(v) = HeaderValue::from_str(&get_content_type(&filepath)) {
                                resp.headers_mut().insert("content-type", v);
                            }
                            return Ok(resp);
                        }
                    }
                }
            }

            // Fallback to directory listing
            if !is_cached {
                return Err(AppError::not_found(format!(
                    "Path not found: {filepath}. Package not yet cached."
                )));
            }

            match get_directory_listing(
                &storage,
                &cache_base,
                &filepath,
                &package_name,
                &resolved.version,
            )
            .await
            {
                Some(listing) => {
                    let body = serde_json::to_string(&listing)?;
                    Ok((
                        StatusCode::OK,
                        [
                            ("content-type", "application/json"),
                            ("cache-control", CDN_CACHE_LONG),
                            ("vary", "Accept-Encoding"),
                            ("x-resolved-version", &resolved.version),
                        ],
                        body,
                    )
                        .into_response())
                }
                None => Err(AppError::not_found(format!("Path not found: {filepath}"))),
            }
        }
    }
}
