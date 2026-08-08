use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::cdn::constants::*;
use crate::cdn::entry::{
    ENTRY_FALLBACKS, resolve_default_file, resolve_esm_entry, resolve_style_file,
};
use crate::cdn::esm::{EsmBundleOptions, bundle_esm_package};
use crate::cdn::listing::{CdnOrgListing, CdnPackageListing, get_directory_listing};
use crate::cdn::minify::minified_entry;
use crate::cdn::registry::fetch_npm_metadata;
use crate::cdn::resolve::{ResolvedVersion, resolve_registry_version};
use crate::cdn::response::file_response_versioned;
use crate::cdn::tarball::{
    cache_package_from_bytes, cache_package_from_tarball, download_tarball,
    extract_file_from_tarball, extract_file_from_tgz, is_package_cached,
};
use crate::error::AppError;
use crate::storage::{CacheMeta, SharedStorage};

static SCOPED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

static NORMAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

static ORG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@([^/]+)/?$").unwrap());

/// Parsed `/cdn/npm/{path}` — either an org listing (`@scope` / `@scope/`) or a
/// concrete package with an optional version and sub-path.
enum ParsedNpmPath {
    OrgListing { scope: String },
    Package {
        name: String,
        version: String,
        filepath: String,
    },
}

/// Shared, read-only context borrowed by every `serve_*` branch: the package state
/// computed once in `handle_npm`, gathered so no field (header, cache-control, cached
/// meta) can be silently dropped between branches.
struct NpmCtx<'a> {
    storage: &'a SharedStorage,
    headers: &'a HeaderMap,
    package_name: &'a str,
    version: &'a str,
    filepath: &'a str,
    cache_base: &'a str,
    tarball_url: &'a str,
    cache_control: &'static str,
    metadata: &'a serde_json::Value,
    resolved: &'a ResolvedVersion,
    cached_meta: &'a Option<CacheMeta>,
    is_cached: bool,
}

fn parse_npm_path(path: &str) -> Result<ParsedNpmPath, AppError> {
    if path.starts_with('@') {
        // Org listing: @scope or @scope/
        if let Some(caps) = ORG_RE.captures(path) {
            return Ok(ParsedNpmPath::OrgListing {
                scope: caps[1].to_string(),
            });
        }
        let caps = SCOPED_RE
            .captures(path)
            .ok_or_else(|| AppError::bad_request("Invalid scoped package path format"))?;
        let scope = &caps[1];
        let pkg = &caps[2];
        let ver = caps.get(3).map(|m| m.as_str()).unwrap_or("latest");
        let fp = caps.get(4).map(|m| m.as_str()).unwrap_or("");
        Ok(ParsedNpmPath::Package {
            name: format!("@{scope}/{pkg}"),
            version: ver.to_string(),
            filepath: fp.to_string(),
        })
    } else {
        let caps = NORMAL_RE
            .captures(path)
            .ok_or_else(|| AppError::bad_request("Invalid package path format"))?;
        let pkg = &caps[1];
        let ver = caps.get(2).map(|m| m.as_str()).unwrap_or("latest");
        let fp = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        Ok(ParsedNpmPath::Package {
            name: pkg.to_string(),
            version: ver.to_string(),
            filepath: fp.to_string(),
        })
    }
}

pub async fn handle_npm(
    State((storage, _)): State<(SharedStorage, crate::winget::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let original_url = uri.to_string();
    let has_trailing_slash = original_url.ends_with('/');

    let (package_name, version, filepath) = match parse_npm_path(&path)? {
        ParsedNpmPath::OrgListing { scope } => {
            let packages = crate::cdn::registry::fetch_org_packages(&storage, &scope)
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
        ParsedNpmPath::Package {
            name,
            version,
            filepath,
        } => (name, version, filepath),
    };

    // Fetch metadata + resolve version (shared by all package branches).
    let metadata = fetch_npm_metadata(&storage, &package_name)
        .await
        .map_err(|_| AppError::not_found("Package not found"))?;
    let resolved = resolve_registry_version(&metadata, &version)
        .ok_or_else(|| AppError::not_found("Version not found"))?;

    // Reject oversized packages before downloading anything. npm metadata carries
    // dist.unpackedSize, so we skip the tarball fetch rather than downloading tens of
    // MB only to discard it when the unpacked size exceeds CDN_MAX_PACKAGE_SIZE
    // (e.g. aws-sdk ~94MB, @tensorflow/tfjs ~141MB).
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
    // short tag TTL. `version == resolved.version` holds iff resolve matched verbatim.
    let cacheable = version == resolved.version;
    let cache_control = if cacheable {
        CDN_CACHE_LONG
    } else {
        CDN_CACHE_TAG
    };
    let cached_meta = is_package_cached(&storage, &cache_base, cacheable).await;
    let is_cached = cached_meta.is_some();

    let ctx = NpmCtx {
        storage: &storage,
        headers: &headers,
        package_name: &package_name,
        version: &version,
        filepath: &filepath,
        cache_base: &cache_base,
        tarball_url: &tarball_url,
        cache_control,
        metadata: &metadata,
        resolved: &resolved,
        cached_meta: &cached_meta,
        is_cached,
    };

    match filepath.as_str() {
        "+esm" => serve_esm_bundle(&ctx).await,
        "" if has_trailing_slash => serve_root_listing(&ctx).await,
        "" => serve_entry_file(&ctx).await,
        _ => serve_subpath(&ctx).await,
    }
}

/// `+esm`: bundle the package entry to a browser-native ESM module.
async fn serve_esm_bundle(ctx: &NpmCtx<'_>) -> Result<Response, AppError> {
    let entry_file =
        resolve_esm_entry(&ctx.resolved.version_info).unwrap_or_else(|| "index.js".to_string());

    if !ctx.is_cached {
        cache_package_from_tarball(
            ctx.storage,
            ctx.tarball_url,
            ctx.cache_base,
            &format!("npm:{}@{}", ctx.package_name, ctx.resolved.version),
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;
    }

    let code = bundle_esm_package(
        ctx.storage,
        &EsmBundleOptions {
            package_name: ctx.package_name.to_string(),
            version: ctx.resolved.version.clone(),
            entry_point: entry_file.clone(),
        },
    )
    .await
    .map_err(|e| AppError::bad_gateway(e.to_string()))?;

    // jsDelivr: an exact version is immutable (1yr); a latest/range alias can move to a
    // new version, so clients must revalidate — never mark an alias immutable.
    Ok(file_response_versioned(
        &entry_file,
        code.as_bytes(),
        ctx.cache_control,
        ctx.headers,
        &ctx.resolved.version,
        None,
    ))
}

/// Root with a trailing slash: directory listing JSON.
async fn serve_root_listing(ctx: &NpmCtx<'_>) -> Result<Response, AppError> {
    if !ctx.is_cached {
        cache_package_from_tarball(
            ctx.storage,
            ctx.tarball_url,
            ctx.cache_base,
            &format!("npm:{}@{}", ctx.package_name, ctx.resolved.version),
        )
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;
    }

    let listing = get_directory_listing(
        ctx.storage,
        ctx.cache_base,
        "",
        ctx.package_name,
        &ctx.resolved.version,
    )
    .await;

    let body = serde_json::to_string(&listing.unwrap_or(CdnPackageListing {
        name: Some(ctx.package_name.to_string()),
        version: Some(ctx.resolved.version.clone()),
        path: String::new(),
        files: vec![],
    }))?;

    Ok((
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", ctx.cache_control),
            ("vary", "Accept-Encoding"),
            ("x-resolved-version", &ctx.resolved.version),
        ],
        body,
    )
        .into_response())
}

/// Root without a trailing slash: the default entry file (jsDelivr priority, always
/// minified). Cached packages read candidates from storage; cold packages download the
/// tarball once and warm the full package in the background reusing those bytes.
async fn serve_entry_file(ctx: &NpmCtx<'_>) -> Result<Response, AppError> {
    // jsDelivr priority (jsdelivr > browser > main, then CSS `style`), then common
    // fallback filenames tried against the actual package contents.
    let entry_candidates = resolve_default_file(&ctx.resolved.version_info)
        .or_else(|| resolve_style_file(&ctx.resolved.version_info))
        .into_iter()
        .chain(ENTRY_FALLBACKS.iter().map(|s| (*s).to_string()));

    let (entry_file, original) = if ctx.is_cached {
        // Filter candidates against the cached file list — reuse the meta
        // is_package_cached already loaded, instead of probing each candidate with its
        // own get_raw round-trip (or a second get_meta).
        let file_names: HashSet<String> = ctx
            .cached_meta
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
        let entry_file = chosen.ok_or_else(|| AppError::not_found("Entry file not found"))?;
        let original = ctx
            .storage
            .get_raw(&format!("{}/{entry_file}", ctx.cache_base))
            .await
            .ok_or_else(|| AppError::not_found("Entry file not found"))?;
        (entry_file, original)
    } else {
        let bytes = download_tarball(ctx.tarball_url)
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
        let s = ctx.storage.clone();
        let b = ctx.cache_base.to_string();
        let l = format!("npm:{}@{}", ctx.package_name, ctx.resolved.version);
        tokio::spawn(async move {
            let _ = cache_package_from_bytes(&s, bytes, &b, &l).await;
        });
        (entry_file, original)
    };

    // jsDelivr: the default file is always minified. `minified_entry` caches the result
    // under a "+min/" suffix so repeated entry requests skip the oxc/lightningcss pass.
    let file_data = minified_entry(ctx.storage, ctx.cache_base, &entry_file, &original).await;
    Ok(file_response_versioned(
        &entry_file,
        &file_data,
        ctx.cache_control,
        ctx.headers,
        &ctx.resolved.version,
        None,
    ))
}

/// Sub-path file: serve the exact file, with `.min` synthesis, jsDelivr version
/// fallback, and a directory-listing fallback for folder paths.
async fn serve_subpath(ctx: &NpmCtx<'_>) -> Result<Response, AppError> {
    // For a cold package, extract_file_from_tarball warms the full package in the
    // background reusing the downloaded bytes, so no separate spawn (and no second
    // tarball download) is needed here.
    let warm_label = format!("npm:{}@{}", ctx.package_name, ctx.resolved.version);
    let warm = (!ctx.is_cached).then_some((ctx.cache_base, warm_label.as_str()));
    match extract_file_from_tarball(
        ctx.storage,
        ctx.tarball_url,
        ctx.filepath,
        &format!("{}/{}", ctx.cache_base, ctx.filepath),
        None,
        warm,
    )
    .await
    {
        Ok(file_data) => {
            // Reuse the cached per-file integrity as the ETag (the meta was already
            // loaded by is_package_cached) instead of re-hashing.
            let etag = ctx
                .cached_meta
                .as_ref()
                .and_then(|m| m.files.as_ref())
                .and_then(|files| files.iter().find(|f| f.name == ctx.filepath))
                .and_then(|f| f.integrity.as_deref());
            Ok(file_response_versioned(
                ctx.filepath,
                &file_data,
                ctx.cache_control,
                ctx.headers,
                &ctx.resolved.version,
                etag,
            ))
        }
        Err(_) => {
            // jsDelivr `.min` synthesis: foo.min.js requested when only foo.js exists,
            // fetched on demand. Returns None for non-.min paths or a missing source,
            // then we fall through to the version retry below.
            if let Some(resp) = crate::cdn::minify::try_min_synthesis(
                ctx.storage,
                ctx.tarball_url,
                ctx.cache_base,
                ctx.filepath,
                ctx.headers,
                ctx.cache_control,
                None,
                Some(&ctx.resolved.version),
            )
            .await?
            {
                return Ok(resp);
            }

            // Version fallback (jsDelivr): the newest version matching the range lacks
            // this file — try the next matching versions before giving up.
            if ctx.version != ctx.resolved.version {
                let candidates =
                    crate::cdn::resolve::resolve_registry_versions_desc(ctx.metadata, ctx.version);
                for cand in candidates.into_iter().skip(1).take(2) {
                    if let Some(info) = ctx.metadata["versions"].get(cand.as_str())
                        && let Some(tb) = info["dist"]["tarball"].as_str()
                    {
                        let cand_base = format!("cdn/npm/{}/{cand}", ctx.package_name);
                        if let Ok(data) = extract_file_from_tarball(
                            ctx.storage,
                            tb,
                            ctx.filepath,
                            &format!("{cand_base}/{}", ctx.filepath),
                            None,
                            None,
                        )
                        .await
                        {
                            return Ok(file_response_versioned(
                                ctx.filepath,
                                &data,
                                ctx.cache_control,
                                ctx.headers,
                                &cand,
                                None,
                            ));
                        }
                    }
                }
            }

            // Fallback to directory listing
            if !ctx.is_cached {
                return Err(AppError::not_found(format!(
                    "Path not found: {}. Package not yet cached.",
                    ctx.filepath
                )));
            }

            match get_directory_listing(
                ctx.storage,
                ctx.cache_base,
                ctx.filepath,
                ctx.package_name,
                &ctx.resolved.version,
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
                            ("x-resolved-version", &ctx.resolved.version),
                        ],
                        body,
                    )
                        .into_response())
                }
                None => Err(AppError::not_found(format!(
                    "Path not found: {}",
                    ctx.filepath
                ))),
            }
        }
    }
}
