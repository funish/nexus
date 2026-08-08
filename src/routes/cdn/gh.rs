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

use crate::cdn::constants::*;
use crate::cdn::listing::{CdnPackageListing, get_directory_listing};
use crate::cdn::minify::minify_for;
use crate::cdn::registry::fetch_github_tags;
use crate::cdn::resolve::resolve_from_tags;
use crate::cdn::response::file_response;
use crate::cdn::tarball::{
    cache_package_from_tarball, extract_file_from_tarball, is_package_cached,
};
use crate::error::AppError;
use crate::storage::{CacheMeta, SharedStorage};

static GH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/]+)/([^@/]+)(?:@([^/]+))?(?:/(.*))?$").unwrap());

/// Parsed `/cdn/gh/{path}`: owner/repo with an optional version request and sub-path.
struct ParsedGhPath {
    owner: String,
    repo: String,
    version_req: String,
    filepath: String,
}

/// Shared, read-only context borrowed by `serve_gh_root`/`serve_gh_subpath`: the
/// resolved repo state computed once in `handle_gh`.
struct GhCtx<'a> {
    storage: &'a SharedStorage,
    headers: &'a HeaderMap,
    filepath: &'a str,
    resolved_version: &'a str,
    cache_base: &'a str,
    tarball_url: &'a str,
    raw_base: &'a str,
    repo_name: &'a str,
    cache_label: &'a str,
    cache_control: &'static str,
    cached_meta: &'a Option<CacheMeta>,
    is_cached: bool,
    warm: Option<(&'a str, &'a str)>,
}

fn parse_gh_path(path: &str) -> Result<ParsedGhPath, AppError> {
    let caps = GH_RE
        .captures(path)
        .ok_or_else(|| AppError::bad_request("Invalid GitHub repository path format"))?;
    Ok(ParsedGhPath {
        owner: caps[1].to_string(),
        repo: caps[2].to_string(),
        version_req: caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string(),
        filepath: caps.get(4).map(|m| m.as_str()).unwrap_or("").to_string(),
    })
}

/// Resolve version via jsDelivr tags (exact -> range -> latest). An API failure is
/// tolerated upstream (`unwrap_or_default`), so an empty version resolves to "main".
/// Returns `(resolved_version, is_semver_tag)`.
fn resolve_gh_version(tags: &[String], version_req: &str) -> (String, bool) {
    if version_req.is_empty() {
        match resolve_from_tags(tags, "*") {
            Some(v) => (v, true),
            None => ("main".to_string(), false),
        }
    } else {
        match resolve_from_tags(tags, version_req) {
            Some(v) => {
                let ok = Version::parse(&v).is_ok();
                (v, ok)
            }
            None => (version_req.to_string(), Version::parse(version_req).is_ok()),
        }
    }
}

/// Build the codeload tarball URL. A full commit hash hits the raw archive; a semver
/// tag uses the tags ref; anything else is treated as a branch ref.
fn build_gh_tarball_url(
    owner: &str,
    repo: &str,
    resolved_version: &str,
    is_hash: bool,
    is_semver: bool,
) -> String {
    if is_hash {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{resolved_version}")
    } else if is_semver {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/tags/{resolved_version}")
    } else {
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{resolved_version}")
    }
}

pub async fn handle_gh(
    State((storage, _)): State<(SharedStorage, crate::winget::db::SharedDb)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let has_trailing_slash = uri.to_string().ends_with('/');

    let ParsedGhPath {
        owner,
        repo,
        version_req,
        filepath,
    } = parse_gh_path(&path)?;

    let tags = fetch_github_tags(&storage, &owner, &repo)
        .await
        .unwrap_or_default();
    let (resolved_version, is_semver) = resolve_gh_version(&tags, &version_req);

    // Full commit hash -> raw archive; semver -> tag ref; otherwise -> branch ref.
    let is_hash = resolved_version.len() == 40
        && resolved_version.chars().all(|c| c.is_ascii_hexdigit());
    let tarball_url = build_gh_tarball_url(&owner, &repo, &resolved_version, is_hash, is_semver);

    let cache_base = format!("cdn/gh/{owner}/{repo}/{resolved_version}");
    let cached_meta = is_package_cached(&storage, &cache_base, is_semver).await;
    let is_cached = cached_meta.is_some();
    // jsDelivr 3-tier cache: exact version/commit hash -> 1yr (immutable);
    // range/latest alias -> 7d; branch ref -> 12h.
    let is_exact_version = !version_req.is_empty() && Version::parse(&version_req).is_ok();
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

    let ctx = GhCtx {
        storage: &storage,
        headers: &headers,
        filepath: &filepath,
        resolved_version: &resolved_version,
        cache_base: &cache_base,
        tarball_url: &tarball_url,
        raw_base: &raw_base,
        repo_name: &repo_name,
        cache_label: &cache_label,
        cache_control,
        cached_meta: &cached_meta,
        is_cached,
        warm,
    };

    if filepath.is_empty() {
        serve_gh_root(&ctx, has_trailing_slash).await
    } else {
        serve_gh_subpath(&ctx).await
    }
}

/// Repository root: a trailing slash serves a directory listing; otherwise README.md
/// falling back to index.js (the default file, always minified when it is JS).
async fn serve_gh_root(ctx: &GhCtx<'_>, has_trailing_slash: bool) -> Result<Response, AppError> {
    // Trailing slash -> directory listing (ensure the package is cached first).
    if has_trailing_slash {
        if !ctx.is_cached {
            cache_package_from_tarball(
                ctx.storage,
                ctx.tarball_url,
                ctx.cache_base,
                ctx.cache_label,
            )
            .await
            .map_err(|e| AppError::bad_gateway(e.to_string()))?;
        }
        let listing = get_directory_listing(
            ctx.storage,
            ctx.cache_base,
            "",
            ctx.repo_name,
            ctx.resolved_version,
        )
        .await
        .unwrap_or(CdnPackageListing {
            name: Some(ctx.repo_name.to_string()),
            version: Some(ctx.resolved_version.to_string()),
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
    let readme_url = format!("{}/README.md", ctx.raw_base);
    let readme_key = format!("{}/README.md", ctx.cache_base);
    if let Ok(data) = extract_file_from_tarball(
        ctx.storage,
        ctx.tarball_url,
        "README.md",
        &readme_key,
        Some(&readme_url),
        ctx.warm,
    )
    .await
    {
        maybe_cache(
            ctx.storage,
            ctx.tarball_url,
            ctx.cache_base,
            ctx.cache_label,
            ctx.is_cached,
        );
        return Ok(file_response(
            "README.md",
            &data,
            ctx.cache_control,
            ctx.headers,
            None,
        ));
    }

    let index_url = format!("{}/index.js", ctx.raw_base);
    let index_key = format!("{}/index.js", ctx.cache_base);
    match extract_file_from_tarball(
        ctx.storage,
        ctx.tarball_url,
        "index.js",
        &index_key,
        Some(&index_url),
        ctx.warm,
    )
    .await
    {
        Ok(data) => {
            maybe_cache(
                ctx.storage,
                ctx.tarball_url,
                ctx.cache_base,
                ctx.cache_label,
                ctx.is_cached,
            );
            // jsDelivr: the default file is always minified. README above is markdown
            // (minify_for passes it through); index.js is real JS — minify it.
            let data = minify_for("index.js", &data);
            Ok(file_response(
                "index.js",
                &data,
                ctx.cache_control,
                ctx.headers,
                None,
            ))
        }
        Err(_) => Err(AppError::not_found(
            "No entry file found (README.md or index.js)",
        )),
    }
}

/// Sub-path file with `.min` synthesis and a directory-listing fallback on 404.
async fn serve_gh_subpath(ctx: &GhCtx<'_>) -> Result<Response, AppError> {
    let file_url = format!("{}/{}", ctx.raw_base, ctx.filepath);
    match extract_file_from_tarball(
        ctx.storage,
        ctx.tarball_url,
        ctx.filepath,
        &format!("{}/{}", ctx.cache_base, ctx.filepath),
        Some(&file_url),
        ctx.warm,
    )
    .await
    {
        Ok(file_data) => {
            maybe_cache(
                ctx.storage,
                ctx.tarball_url,
                ctx.cache_base,
                ctx.cache_label,
                ctx.is_cached,
            );
            // Reuse the cached per-file integrity as the ETag (the meta was already
            // loaded by is_package_cached) instead of re-hashing.
            let etag = ctx
                .cached_meta
                .as_ref()
                .and_then(|m| m.files.as_ref())
                .and_then(|files| files.iter().find(|f| f.name == ctx.filepath))
                .and_then(|f| f.integrity.as_deref());
            Ok(file_response(
                ctx.filepath,
                &file_data,
                ctx.cache_control,
                ctx.headers,
                etag,
            ))
        }
        Err(_) => {
            // jsDelivr `.min` synthesis: foo.min.js requested when only foo.js exists,
            // fetched on demand. Returns None for non-.min paths or a missing source,
            // then we fall through to the cached/404 branch below.
            if let Some(resp) = crate::cdn::minify::try_min_synthesis(
                ctx.storage,
                ctx.tarball_url,
                ctx.cache_base,
                ctx.filepath,
                ctx.headers,
                ctx.cache_control,
                Some(ctx.raw_base),
                None,
            )
            .await?
            {
                return Ok(resp);
            }

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
                ctx.repo_name,
                ctx.resolved_version,
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
                None => Err(AppError::not_found(format!(
                    "Path not found: {}",
                    ctx.filepath
                ))),
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
