//! GitHub Trees API access with TTL caching + single-flight (mirrors winget/tree.ts).
//!
//! Discovers manifest file paths under `manifests/<letter>/...` and caches the
//! letter-directory SHAs and tree paths with a 10-minute TTL. Concurrent cache
//! misses for the same key share one API call (single-flight), so a burst of
//! packageManifests builds fires one tree request per key — not one per build,
//! which would burn the GitHub budget (60/h anonymous, 5000/h authenticated).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::storage::SharedStorage;
use crate::utils::cache::{cache_fresh, set_mtime};
use crate::utils::concurrency::DOWNLOAD_SEMAPHORE;

use super::constants::*;

#[derive(Debug, Deserialize)]
struct TreeItem {
    path: String,
    #[allow(dead_code)]
    mode: String,
    #[serde(rename = "type")]
    item_type: String,
    sha: String,
    #[allow(dead_code)]
    url: String,
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    #[allow(dead_code)]
    sha: String,
    tree: Vec<TreeItem>,
}

/// Fetch a GitHub tree by SHA or branch (mirrors getGitHubTree). Goes through
/// the shared retry-capable client (429/5xx backoff + GITHUB_TOKEN) and the
/// download semaphore, so the GitHub budget is respected.
async fn get_github_tree(tree_sha: &str, recursive: bool) -> Result<TreeResponse> {
    let _permit = DOWNLOAD_SEMAPHORE.acquire().await.unwrap();
    let url = format!(
        "{WINGET_GITHUB_API_BASE}/repos/{WINGET_GITHUB_REPO}/git/trees/{tree_sha}{}",
        if recursive { "?recursive=1" } else { "" }
    );
    let resp = crate::utils::http::get_with_retry(
        &url,
        Duration::from_secs(30),
        crate::utils::http::GITHUB_TOKEN.as_deref(),
        &[],
    )
    .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch GitHub tree: {}", resp.status());
    }
    Ok(resp.json().await?)
}

/// TTL-cached value with single-flight dedup. Concurrent misses for `key` share
/// one `fetch`: the leader runs it and writes storage; followers wait on the
/// single-flight broadcast, then re-read. Without this, N concurrent
/// packageManifests builds each fire their own tree API request for the same
/// letter/package, wasting the GitHub budget.
async fn cached_singleflight<T>(
    storage: &SharedStorage,
    key: &str,
    fetch: impl std::future::Future<Output = Result<T>>,
) -> Result<T>
where
    T: serde::Serialize + DeserializeOwned,
{
    // Fast path.
    if cache_fresh(storage, key, WINGET_UPDATE_INTERVAL_SECS).await
        && let Some(data) = storage.get_raw(key).await
        && let Ok(v) = serde_json::from_slice::<T>(&data)
    {
        return Ok(v);
    }

    let storage_c = storage.clone();
    let key_c = key.to_string();
    crate::utils::singleflight::run_once(key, move || async move {
        // Double-check after winning leadership — another leader may have just
        // populated the cache while this one waited.
        if cache_fresh(&storage_c, &key_c, WINGET_UPDATE_INTERVAL_SECS).await
            && storage_c.get_raw(&key_c).await.is_some()
        {
            return;
        }
        if let Ok(v) = fetch.await
            && let Ok(bytes) = serde_json::to_vec(&v)
        {
            storage_c.set_raw(&key_c, &bytes).await;
            set_mtime(&storage_c, &key_c).await;
        }
    })
    .await;

    storage
        .get_raw(key)
        .await
        .and_then(|data| serde_json::from_slice::<T>(&data).ok())
        .ok_or_else(|| anyhow::anyhow!("github tree cache miss after single-flight: {key}"))
}

/// Cached, recursive tree file paths (mirrors getGitHubTreePaths).
pub async fn get_github_tree_paths(
    storage: &SharedStorage,
    tree_sha: &str,
    cache_suffix: &str,
) -> Result<Vec<String>> {
    let normalized = cache_suffix.replace('/', "-");
    let cache_key = format!("{WINGET_CACHE_PREFIX}/{normalized}");
    let tree_sha = tree_sha.to_string();
    cached_singleflight(storage, &cache_key, async move {
        let tree = get_github_tree(&tree_sha, true).await?;
        Ok(tree.tree.into_iter().map(|i| i.path).collect())
    })
    .await
}

/// Cached letter-directory SHAs (a-z, 0-9) under manifests/ (mirrors getLetterDirectoryShas).
pub async fn get_letter_directory_shas(storage: &SharedStorage) -> Result<HashMap<String, String>> {
    let cache_key = format!("{WINGET_CACHE_PREFIX}/letter-shas.json");
    cached_singleflight(storage, &cache_key, async {
        let manifests_sha = fetch_manifests_sha(storage).await?;
        let tree = get_github_tree(&manifests_sha, false).await?;
        let mut shas = HashMap::new();
        for item in &tree.tree {
            if item.item_type == "tree"
                && item.path.len() == 1
                && item.path.chars().all(|c| c.is_ascii_alphanumeric())
            {
                shas.insert(item.path.clone(), item.sha.clone());
            }
        }
        if shas.is_empty() {
            anyhow::bail!("No letter directories found in manifests");
        }
        Ok(shas)
    })
    .await
}

/// Cached SHA of the manifests/ directory (mirrors fetchManifestsSha).
pub async fn fetch_manifests_sha(storage: &SharedStorage) -> Result<String> {
    cached_singleflight(storage, WINGET_MANIFESTS_SHA_KEY, async {
        let root = get_github_tree(WINGET_GITHUB_BRANCH, false).await?;
        let manifests = root
            .tree
            .iter()
            .find(|i| i.path == "manifests" && i.item_type == "tree")
            .ok_or_else(|| anyhow::anyhow!("manifests directory not found in repository"))?;
        Ok(manifests.sha.clone())
    })
    .await
}
