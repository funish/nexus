use anyhow::Result;
use serde_json::Value;

use super::cache::{META_CACHE_TTL_SECS, cached_json};
use super::constants::{CDN_FETCH_TIMEOUT_SECS, CDN_JSR_REGISTRY, CDN_NPM_REGISTRY};
use crate::storage::SharedStorage;

pub async fn fetch_npm_metadata(storage: &SharedStorage, package_name: &str) -> Result<Value> {
    cached_json(
        storage,
        &format!("registry/npm/{package_name}"),
        META_CACHE_TTL_SECS,
        async {
            let url = format!("{CDN_NPM_REGISTRY}/{package_name}");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
                .build()?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Package not found: {package_name}");
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
    cached_json(
        storage,
        &format!("registry/jsr/{scope}/{package}"),
        META_CACHE_TTL_SECS,
        async {
            let npm_name = format!("@jsr/{}__{}", scope, package);
            let url = format!("{CDN_JSR_REGISTRY}/{npm_name}");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
                .build()?;
            let resp = client.get(&url).send().await?;
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
    cached_json(
        storage,
        &format!("registry/gh/{owner}/{repo}/tags"),
        META_CACHE_TTL_SECS,
        async {
            let url = format!("https://data.jsdelivr.com/v1/packages/gh/{owner}/{repo}");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
                .build()?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("GitHub repo not found: {owner}/{repo}");
            }
            let data: Value = resp.json().await?;
            Ok::<Vec<String>, anyhow::Error>(
                data["versions"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v["version"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        },
    )
    .await
}

pub async fn fetch_cdnjs_library(storage: &SharedStorage, library: &str) -> Result<Value> {
    cached_json(
        storage,
        &format!("registry/cdnjs/{library}"),
        META_CACHE_TTL_SECS,
        async {
            let url = format!(
                "https://api.cdnjs.com/libraries/{library}?fields=version,versions,filename"
            );
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
                .build()?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("cdnjs library not found: {library}");
            }
            Ok(resp.json::<Value>().await?)
        },
    )
    .await
}

pub async fn fetch_cdnjs_files(library: &str, version: &str) -> Result<Value> {
    let _permit = super::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .unwrap();
    let url = format!("https://api.cdnjs.com/libraries/{library}/{version}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
        .build()?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("cdnjs version not found: {library}@{version}");
    }
    Ok(resp.json().await?)
}

pub async fn fetch_org_packages(storage: &SharedStorage, scope: &str) -> Result<Vec<String>> {
    cached_json(
        storage,
        &format!("registry/org/{scope}"),
        META_CACHE_TTL_SECS,
        async {
            let url = format!("{CDN_NPM_REGISTRY}/-/org/{scope}/package");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
                .build()?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Organization not found: @{scope}");
            }
            let data: serde_json::Map<String, Value> = resp.json().await?;
            Ok::<Vec<String>, anyhow::Error>(data.keys().cloned().collect())
        },
    )
    .await
}
