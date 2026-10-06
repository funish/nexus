use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use super::constants::{
    CDN_FETCH_TIMEOUT_SECS, cdnjs_api_base, github_api_base, jsr_registry, npm_registry,
};
use crate::storage::SharedStorage;
use crate::utils::cache::{META_CACHE_TTL_SECS, cached_json};
use crate::utils::http::{GITHUB_TOKEN, get_with_retry};

/// Per-request timeout for registry metadata fetches.
const FETCH_TIMEOUT: Duration = Duration::from_secs(CDN_FETCH_TIMEOUT_SECS);

/// Abbreviated packument ("corgi doc"): strips readme/description/keywords/time/
/// author/maintainers — everything we don't read — while keeping versions,
/// dist-tags, and dist.tarball/unpackedSize, which is all resolve/esm use. Cuts
/// large packuments by an order of magnitude, so the per-request Value parse
/// stays proportional to install data instead of human-facing metadata.
const NPM_ABBREVIATED_ACCEPT: &str = "application/vnd.npm.install-v1+json";

/// Marker for a registry outage (429/5xx after retries) as opposed to a package
/// that genuinely doesn't exist. Callers map it to 502 so transient upstream
/// failures aren't cacheable 404s (jsDelivr treats those the same way).
#[derive(Debug, thiserror::Error)]
#[error("registry upstream error: HTTP {0}")]
pub struct RegistryUpstreamError(pub u16);

pub async fn fetch_npm_metadata(storage: &SharedStorage, package_name: &str) -> Result<Value> {
    let package_name = package_name.to_string();
    cached_json(
        storage,
        &format!("registry/npm/{package_name}"),
        META_CACHE_TTL_SECS,
        async move {
            let url = format!("{}/{package_name}", npm_registry());
            let resp = get_with_retry(&url, FETCH_TIMEOUT, None, &[("Accept", NPM_ABBREVIATED_ACCEPT)]).await?;
            let status = resp.status();
            if !status.is_success() {
                if status.as_u16() == 404 {
                    anyhow::bail!("Package not found: {package_name}");
                }
                tracing::warn!("npm registry upstream error: HTTP {status} for {package_name}");
                return Err(RegistryUpstreamError(status.as_u16()).into());
            }
            Ok(resp.json::<Value>().await?)
        },
    )
    .await
}

pub async fn fetch_jsr_metadata(
    storage: &SharedStorage,
    scope: &str,
    package: &str,
) -> Result<Value> {
    let scope = scope.to_string();
    let package = package.to_string();
    cached_json(
        storage,
        &format!("registry/jsr/{scope}/{package}"),
        META_CACHE_TTL_SECS,
        async move {
            let npm_name = format!("@jsr/{}__{}", scope, package);
            let url = format!("{}/{npm_name}", jsr_registry());
            let resp = get_with_retry(&url, FETCH_TIMEOUT, None, &[]).await?;
            if !resp.status().is_success() {
                anyhow::bail!("JSR package not found: @{scope}/{package}");
            }
            Ok(resp.json::<Value>().await?)
        },
    )
    .await
}

pub async fn fetch_github_tags(
    storage: &SharedStorage,
    owner: &str,
    repo: &str,
) -> Result<Vec<String>> {
    let owner = owner.to_string();
    let repo = repo.to_string();
    cached_json(
        storage,
        &format!("registry/gh/{owner}/{repo}/tags"),
        META_CACHE_TTL_SECS,
        async move {
            // GitHub tags API returns the *original* tag names (e.g. "v5.3.3"), which
            // raw.githubusercontent.com and codeload refs require. The jsDelivr
            // packages API normalizes away the "v" prefix and would 404 against GitHub
            // when building tarball/raw URLs.
            let url = format!(
                "{}/repos/{owner}/{repo}/tags?per_page=100",
                github_api_base()
            );
            let resp = get_with_retry(
                &url,
                FETCH_TIMEOUT,
                GITHUB_TOKEN.as_deref(),
                &[("Accept", "application/vnd.github+json")],
            )
            .await?;
            if !resp.status().is_success() {
                anyhow::bail!("GitHub repo not found: {owner}/{repo}");
            }
            let data: Value = resp.json().await?;
            Ok::<Vec<String>, anyhow::Error>(
                data.as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        },
    )
    .await
}

pub async fn fetch_cdnjs_library(storage: &SharedStorage, library: &str) -> Result<Value> {
    let library = library.to_string();
    cached_json(
        storage,
        &format!("registry/cdnjs/{library}"),
        META_CACHE_TTL_SECS,
        async move {
            let url = format!(
                "{}/libraries/{library}?fields=version,versions,filename",
                cdnjs_api_base()
            );
            let resp = get_with_retry(&url, FETCH_TIMEOUT, None, &[]).await?;
            if !resp.status().is_success() {
                anyhow::bail!("cdnjs library not found: {library}");
            }
            Ok(resp.json::<Value>().await?)
        },
    )
    .await
}

pub async fn fetch_cdnjs_files(library: &str, version: &str) -> Result<Value> {
    let _permit = crate::utils::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .unwrap();
    let url = format!("{}/libraries/{library}/{version}", cdnjs_api_base());
    let resp = get_with_retry(&url, FETCH_TIMEOUT, None, &[]).await?;
    if !resp.status().is_success() {
        anyhow::bail!("cdnjs version not found: {library}@{version}");
    }
    Ok(resp.json().await?)
}

pub async fn fetch_org_packages(storage: &SharedStorage, scope: &str) -> Result<Vec<String>> {
    let scope = scope.to_string();
    cached_json(
        storage,
        &format!("registry/org/{scope}"),
        META_CACHE_TTL_SECS,
        async move {
            let url = format!("{}/-/org/{scope}/package", npm_registry());
            let resp = get_with_retry(&url, FETCH_TIMEOUT, None, &[]).await?;
            if !resp.status().is_success() {
                anyhow::bail!("Organization not found: @{scope}");
            }
            let data: serde_json::Map<String, Value> = resp.json().await?;
            Ok::<Vec<String>, anyhow::Error>(data.keys().cloned().collect())
        },
    )
    .await
}
