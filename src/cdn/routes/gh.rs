//! GitHub releases/repo CDN route — mirrors gh/[...path].ts.
//!
//! Resolves owner/repo@version via jsDelivr tags, serves files from the GitHub
//! tarball (with a raw.githubusercontent.com single-file fast path), and exposes
//! directory listings on a trailing-slash root plus a 404 directory fallback.

use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use node_semver::Version;
use regex::Regex;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::listing::{CdnPackageListing, get_directory_listing};
use crate::cdn::utils::minify::{minify_for, strip_min_suffix};
use crate::cdn::utils::registry::fetch_github_tags;
use crate::cdn::utils::resolve::resolve_from_tags;
use crate::cdn::utils::response::file_response;
use crate::cdn::utils::tarball::{
    cache_package_from_tarball, extract_file_from_tarball, is_package_cached,
};
use crate::error::AppError;
use crate::storage::SharedStorage;

static GH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

pub async fn handle_gh(
    State((storage, _)): State<(SharedStorage, crate::winget::utils::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let has_trailing_slash = uri.to_string().ends_with('/');

    let caps = GH_RE
        .captures(&path)
        .ok_or_else(|| AppError::bad_request("Invalid GitHub repository path format"))?;

    let owner = &caps[1];
    let repo = &caps[2];
    let version_req = caps.get(3).map(|m| m.as_str()).unwrap_or("");
    let filepath = caps.get(4).map(|m| m.as_str()).unwrap_or("");

    // Resolve version via jsDelivr tags (exact -> range -> latest). An API failure is
    // tolerated so the request still proceeds with the version as given (or "main").
    let tags = fetch_github_tags(&storage, owner, repo)
        .await
        .unwrap_or_default();
    let (resolved_version, is_semver) = if version_req.is_empty() {
        match resolve_from_tags(&tags, "*") {
            Some(v) => (v, true),
            None => ("main".to_string(), false),
        }
    } else {
        match resolve_from_tags(&tags, version_req) {
            Some(v) => {
                let ok = Version::parse(&v).is_ok();
                (v, ok)
            }
            None => (version_req.to_string(), Version::parse(version_req).is_ok()),
        }
    };

    // Full commit hash -> raw archive; semver -> tag ref; otherwise -> branch ref.
    let is_hash =
        resolved_version.len() == 40 && resolved_version.chars().all(|c| c.is_ascii_hexdigit());
    let tarball_url = if is_hash {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{resolved_version}")
    } else if is_semver {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/tags/{resolved_version}")
    } else {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{resolved_version}")
    };

    let cache_base = format!("cdn/gh/{owner}/{repo}/{resolved_version}");
    let cached_meta = is_package_cached(&storage, &cache_base, is_semver).await;
    let is_cached = cached_meta.is_some();
    // jsDelivr 3-tier cache: exact version/commit hash -> 1yr (immutable);
    // range/latest alias -> 7d; branch ref -> 12h.
    let is_exact_version = !version_req.is_empty() && Version::parse(version_req).is_ok();
    let cache_control = if is_hash || is_exact_version {
        CDN_CACHE_LONG
    } else if is_semver {
        CDN_CACHE_TAG
    } else {
        CDN_CACHE_BRANCH
    };
    let raw_base = format!("https://raw.githubusercontent.com/{owner}/{repo}/{resolved_version}");
    let repo_name = format!("{owner}/{repo}");
    let cache_label = format!("gh:{repo_name}@{resolved_version}");
    // When the raw fast path misses and we fall back to downloading the tarball, warm
    // the full package reusing those bytes. maybe_cache still covers the direct_url-hit
    // case (no bytes to reuse then); the PENDING dedup in cache_package_* ensures at
    // most one tarball download across both spawns.
    let warm = (!is_cached).then_some((cache_base.as_str(), cache_label.as_str()));

    // Repository root.
    if filepath.is_empty() {
        // Trailing slash -> directory listing (ensure the package is cached first).
        if has_trailing_slash {
            if !is_cached {
                cache_package_from_tarball(&storage, &tarball_url, &cache_base, &cache_label)
                    .await
                    .map_err(|e| AppError::bad_gateway(e.to_string()))?;
            }
            let listing =
                get_directory_listing(&storage, &cache_base, "", &repo_name, &resolved_version)
                    .await
                    .unwrap_or(CdnPackageListing {
                        name: Some(repo_name),
                        version: Some(resolved_version.clone()),
                        path: String::new(),
                        files: vec![],
                    });
            let body = serde_json::to_string(&listing)?;
            return Ok((
                StatusCode::OK,
                [
                    ("content-type", "application/json"),
                    ("cache-control", CDN_CACHE_SHORT),
                    ("vary", "Accept-Encoding"),
                ],
                body,
            )
                .into_response());
        }

        // No trailing slash -> README.md, falling back to index.js, with background caching.
        let readme_url = format!("{raw_base}/README.md");
        let readme_key = format!("{cache_base}/README.md");
        if let Ok(data) = extract_file_from_tarball(
            &storage,
            &tarball_url,
            "README.md",
            &readme_key,
            Some(&readme_url),
            warm,
        )
        .await
        {
            maybe_cache(&storage, &tarball_url, &cache_base, &cache_label, is_cached);
            return Ok(file_response("README.md", &data, cache_control, &headers, None));
        }

        let index_url = format!("{raw_base}/index.js");
        let index_key = format!("{cache_base}/index.js");
        return match extract_file_from_tarball(
            &storage,
            &tarball_url,
            "index.js",
            &index_key,
            Some(&index_url),
            warm,
        )
        .await
        {
            Ok(data) => {
                maybe_cache(&storage, &tarball_url, &cache_base, &cache_label, is_cached);
                // jsDelivr: the default file is always minified. README above is markdown
                // (minify_for passes it through); index.js is real JS — minify it.
                let data = minify_for("index.js", &data);
                Ok(file_response("index.js", &data, cache_control, &headers, None))
            }
            Err(_) => Err(AppError::not_found(
                "No entry file found (README.md or index.js)",
            )),
        };
    }

    // Sub-path file with directory-listing fallback on 404.
    let file_url = format!("{raw_base}/{filepath}");
    match extract_file_from_tarball(
        &storage,
        &tarball_url,
        filepath,
        &format!("{cache_base}/{filepath}"),
        Some(&file_url),
        warm,
    )
    .await
    {
        Ok(file_data) => {
            maybe_cache(&storage, &tarball_url, &cache_base, &cache_label, is_cached);
            // Reuse the cached per-file integrity as the ETag (the meta was
            // already loaded by is_package_cached) instead of re-hashing.
            let etag = cached_meta
                .as_ref()
                .and_then(|m| m.files.as_ref())
                .and_then(|files| files.iter().find(|f| f.name == filepath))
                .and_then(|f| f.integrity.as_deref());
            Ok(file_response(filepath, &file_data, cache_control, &headers, etag))
        }
        Err(_) => {
            // jsDelivr `.min` synthesis: foo.min.js requested but only foo.js exists.
            // Works for cold packages too: extract_file_from_tarball downloads the
            // tarball and warms it, so the un-minified source is fetched on demand.
            if let Some(orig) = strip_min_suffix(filepath)
                && let Ok(orig_data) = extract_file_from_tarball(
                    &storage,
                    &tarball_url,
                    &orig,
                    &format!("{cache_base}/{orig}"),
                    Some(&format!("{raw_base}/{orig}")),
                    None,
                )
                .await
            {
                let minified = minify_for(&orig, &orig_data);
                let s = storage.clone();
                let (k, d) = (format!("{cache_base}/{filepath}"), minified.clone());
                tokio::spawn(async move {
                    s.set_raw(&k, &d).await;
                });
                return Ok(file_response(filepath, &minified, cache_control, &headers, None));
            }

            if !is_cached {
                return Err(AppError::not_found(format!(
                    "Path not found: {filepath}. Package not yet cached."
                )));
            }
            match get_directory_listing(
                &storage,
                &cache_base,
                filepath,
                &repo_name,
                &resolved_version,
            )
            .await
            {
                Some(listing) => {
                    let body = serde_json::to_string(&listing)?;
                    Ok((
                        StatusCode::OK,
                        [
                            ("content-type", "application/json"),
                            ("cache-control", CDN_CACHE_SHORT),
                            ("vary", "Accept-Encoding"),
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

/// Trigger background caching of the full package when not already cached
/// (mirrors event.waitUntil in gh/[...path].ts).
fn maybe_cache(
    storage: &SharedStorage,
    tarball_url: &str,
    cache_base: &str,
    label: &str,
    is_cached: bool,
) {
    if is_cached {
        return;
    }
    let s = storage.clone();
    let u = tarball_url.to_string();
    let b = cache_base.to_string();
    let l = label.to_string();
    tokio::spawn(async move {
        let _ = cache_package_from_tarball(&s, &u, &b, &l).await;
    });
}
