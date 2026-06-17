use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::cdn::utils::constants::*;
use crate::cdn::utils::entry::{
    ENTRY_FALLBACKS, resolve_default_file, resolve_esm_entry, resolve_style_file,
};
use crate::cdn::utils::esm::{EsmBundleOptions, bundle_esm_package};
use crate::cdn::utils::listing::{CdnOrgListing, CdnPackageListing, get_directory_listing};
use crate::cdn::utils::minify::{minified_entry, minify_for};
use crate::cdn::utils::registry::fetch_npm_metadata;
use crate::cdn::utils::resolve::resolve_registry_version;
use crate::cdn::utils::response::file_response_versioned;
use crate::cdn::utils::tarball::{
    cache_package_from_bytes, cache_package_from_tarball, download_tarball,
    extract_file_from_tarball, extract_file_from_tgz, is_package_cached,
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

    // Reject oversized packages before downloading anything. npm metadata
    // carries dist.unpackedSize, so we skip the tarball fetch entirely rather
    // than downloading tens of MB only to discard it when the unpacked size
    // exceeds CDN_MAX_PACKAGE_SIZE (e.g. aws-sdk ~94MB, @tensorflow/tfjs ~141MB).
    if let Some(unpacked) = resolved.version_info["dist"]["unpackedSize"].as_u64()
        && unpacked > CDN_MAX_PACKAGE_SIZE
    {
        return Err(AppError::not_found(format!(
            "Package {package_name}@{} is too large to serve ({} bytes unpacked)",
            resolved.version, unpacked
        )));
    }

    let tarball_url = resolved.version_info["dist"]["tarball"]
        .as_str()
        .ok_or_else(|| AppError::bad_gateway("Missing tarball URL"))?
        .to_string();

    let cache_base = format!("cdn/npm/{package_name}/{}", resolved.version);
    // Immutable only when the request named an exact version. A latest/range alias
    // resolves to an exact version but can move later, so jsDelivr caches those at the
    // short tag TTL. `version == resolved.version` holds iff resolve matched the request
    // verbatim; ranges/dist-tags resolve to a different string.
    let cacheable = version == resolved.version;
    let cache_control = if cacheable {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };
    let cached_meta = is_package_cached(&storage, &cache_base, cacheable).await;
    let is_cached = cached_meta.is_some();

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
                entry_point: entry_file.clone(),
            },
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;

        // jsDelivr: an exact version is immutable (1yr); a latest/range alias can
        // move to a new version, so clients must revalidate — never mark an alias
        // immutable.
        return Ok(file_response_versioned(
            &entry_file,
            code.as_bytes(),
            cache_control,
            &headers,
            &resolved.version,
            None,
        ));
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
                    ("cache-control", cache_control),
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

            // For a cached package, read candidates from storage. For a cold package,
            // download the tarball once, return the first candidate present, and warm the
            // full package in the background reusing those bytes — so the response returns
            // as soon as the entry file is extracted, not after caching the whole package.
            let (entry_file, original) = if is_cached {
                // Filter candidates against the cached file list — reuse the meta
                // is_package_cached already loaded, instead of probing each candidate
                // with its own get_raw round-trip (or a second get_meta).
                let file_names: HashSet<String> = cached_meta
                    .as_ref()
                    .and_then(|m| m.files.as_ref())
                    .map(|files| files.iter().map(|f| f.name.clone()).collect())
                    .unwrap_or_default();
                let mut chosen = None;
                for cand in entry_candidates {
                    if file_names.contains(cand.as_str()) {
                        chosen = Some(cand);
                        break;
                    }
                }
                let entry_file =
                    chosen.ok_or_else(|| AppError::not_found("Entry file not found"))?;
                let original = storage
                    .get_raw(&format!("{cache_base}/{entry_file}"))
                    .await
                    .ok_or_else(|| AppError::not_found("Entry file not found"))?;
                (entry_file, original)
            } else {
                let bytes = download_tarball(&tarball_url)
                    .await
                    .map_err(|e| AppError::bad_gateway(e.to_string()))?;
                let mut found = None;
                for cand in entry_candidates {
                    if let Some(data) = extract_file_from_tgz(&bytes, &cand) {
                        found = Some((cand, data));
                        break;
                    }
                }
                let (entry_file, original) =
                    found.ok_or_else(|| AppError::not_found("Entry file not found"))?;
                let s = storage.clone();
                let b = cache_base.clone();
                let l = format!("npm:{package_name}@{}", resolved.version);
                tokio::spawn(async move {
                    let _ = cache_package_from_bytes(&s, bytes, &b, &l).await;
                });
                (entry_file, original)
            };

            // jsDelivr: the default file is always minified. `minified_entry` caches the
            // result under a "+min/" suffix so repeated entry requests skip the
            // oxc/lightningcss pass (full parse + rewrite).
            let file_data = minified_entry(&storage, &cache_base, &entry_file, &original).await;
            return Ok(file_response_versioned(
                &entry_file,
                &file_data,
                cache_control,
                &headers,
                &resolved.version,
                None,
            ));
        }
    }

    // Sub-path file. For a cold package, `extract_file_from_tarball` warms the full
    // package in the background reusing the downloaded bytes, so no separate spawn
    // (and no second tarball download) is needed here.
    let warm_label = format!("npm:{package_name}@{}", resolved.version);
    let warm = (!is_cached).then_some((cache_base.as_str(), warm_label.as_str()));
    match extract_file_from_tarball(
        &storage,
        &tarball_url,
        &filepath,
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
                &filepath,
                &file_data,
                cache_control,
                &headers,
                &resolved.version,
                etag,
            ))
        }
        Err(_) => {
            // jsDelivr `.min` synthesis: foo.min.js requested but only foo.js exists.
            // Works for cold packages too: extract_file_from_tarball downloads the
            // tarball and warms it, so the un-minified source is fetched on demand.
            if let Some(orig) = crate::cdn::utils::minify::strip_min_suffix(&filepath)
                && let Ok(orig_data) = extract_file_from_tarball(
                    &storage,
                    &tarball_url,
                    &orig,
                    &format!("{cache_base}/{orig}"),
                    None,
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

                return Ok(file_response_versioned(
                    &filepath,
                    &minified,
                    cache_control,
                    &headers,
                    &resolved.version,
                    None,
                ));
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
                            None,
                        )
                        .await
                        {
                            return Ok(file_response_versioned(
                                &filepath,
                                &data,
                                cache_control,
                                &headers,
                                &cand,
                                None,
                            ));
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
