//! GitHub Trees API access with stale-while-revalidate (mirrors winget/tree.ts).
//!
//! Discovers manifest file paths under `manifests/<letter>/...` and caches the
//! letter-directory SHAs and tree paths with a 10-minute soft TTL. After expiry,
//! the last good value is returned while one single-flight refresh runs, so a burst
//! of packageManifests builds fires at most one tree request per key — not one per build,
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
    #[serde(default)]
    truncated: bool,
}

/// Fetch a GitHub tree by SHA or branch (mirrors getGitHubTree). Goes through
/// the shared retry-capable client (429/5xx backoff + GITHUB_TOKEN) and the
/// download semaphore, so the GitHub budget is respected.
async fn get_github_tree(tree_sha: &str, recursive: bool) -> Result<TreeResponse> {
    let _permit = DOWNLOAD_SEMAPHORE.acquire().await.unwrap();
    let url = format!(
        "{WINGET_GITHUB_API_BASE}/repos/{}/git/trees/{tree_sha}{}",
        crate::config::winget_github_repo(),
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
    let tree: TreeResponse = resp.json().await?;
    if tree.truncated {
        anyhow::bail!(
            "GitHub tree {tree_sha} is truncated; refusing to cache an incomplete listing"
        );
    }
    Ok(tree)
}

/// Refresh a tree-cache entry through single-flight. Concurrent refreshes for
/// `key` share one `fetch`: the leader runs it and writes storage; followers wait
/// on the single-flight broadcast, then re-read.
async fn refresh_singleflight<T, F>(storage: SharedStorage, key: String, fetch: F)
where
    T: serde::Serialize + DeserializeOwned + Send + 'static,
    F: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let run_key = key.clone();
    crate::utils::singleflight::run_once(&run_key, move || async move {
        // Double-check after winning leadership — another leader may have just
        // populated the cache while this one waited.
        if cache_fresh(&storage, &key, WINGET_UPDATE_INTERVAL_SECS).await
            && storage.get_raw(&key).await.is_some()
        {
            return;
        }
        if let Ok(v) = fetch.await
            && let Ok(bytes) = serde_json::to_vec(&v)
        {
            match storage.set_raw(&key, &bytes).await {
                Ok(()) => set_mtime(&storage, &key).await,
                Err(e) => tracing::warn!("Failed to refresh tree cache {key}: {e}"),
            }
        }
    })
    .await;
}

/// TTL-cached value with stale-while-revalidate. A fresh value is returned
/// directly. A stale value is returned immediately while one background refresh
/// runs; only a complete miss blocks on the fetch. This keeps the first request
/// after the TTL fast without letting concurrent requests duplicate GitHub calls.
async fn cached_singleflight<T, F>(storage: &SharedStorage, key: &str, fetch: F) -> Result<T>
where
    T: serde::Serialize + DeserializeOwned + Send + 'static,
    F: std::future::Future<Output = Result<T>> + Send + 'static,
{
    if cache_fresh(storage, key, WINGET_UPDATE_INTERVAL_SECS).await
        && let Some(data) = storage.get_raw(key).await
        && let Ok(v) = serde_json::from_slice::<T>(&data)
    {
        return Ok(v);
    }

    if let Some(data) = storage.get_raw(key).await
        && let Ok(stale) = serde_json::from_slice::<T>(&data)
    {
        if !crate::utils::singleflight::is_pending(key) {
            let storage = storage.clone();
            let key = key.to_string();
            tokio::spawn(refresh_singleflight(storage, key, fetch));
        }
        return Ok(stale);
    }

    refresh_singleflight(storage.clone(), key.to_string(), fetch).await;
    storage
        .get_raw(key)
        .await
        .and_then(|data| serde_json::from_slice::<T>(&data).ok())
        .ok_or_else(|| anyhow::anyhow!("github tree cache miss after single-flight: {key}"))
}

/// Immortal cache for content-addressed (SHA-keyed) trees: a stored value is
/// trusted forever because the key IS the content. Corrupt entries are refetched
/// instead of failing forever. No TTL, no mtime — unlike branch aliases, a SHA
/// never changes, so refreshing would only burn GitHub API budget.
async fn cached_immutable<T, F>(storage: &SharedStorage, key: &str, fetch: F) -> Result<T>
where
    T: serde::Serialize + DeserializeOwned + Send + 'static,
    F: std::future::Future<Output = Result<T>> + Send + 'static,
{
    if let Some(data) = storage.get_raw(key).await
        && let Ok(v) = serde_json::from_slice::<T>(&data)
    {
        return Ok(v);
    }

    let run_key = key.to_string();
    let fetch_storage = storage.clone();
    let cache_key = key.to_string();
    crate::utils::singleflight::run_once(&run_key, move || async move {
        // Re-check: another waiter may have populated the entry while this one
        // was acquiring leadership.
        if let Some(data) = fetch_storage.get_raw(&cache_key).await
            && serde_json::from_slice::<T>(&data).is_ok()
        {
            return;
        }
        if let Ok(v) = fetch.await
            && let Ok(bytes) = serde_json::to_vec(&v)
            && let Err(e) = fetch_storage.set_raw(&cache_key, &bytes).await
        {
            tracing::warn!("Failed to cache immutable tree {cache_key}: {e}");
        }
    })
    .await;

    storage
        .get_raw(key)
        .await
        .and_then(|data| serde_json::from_slice::<T>(&data).ok())
        .ok_or_else(|| anyhow::anyhow!("immutable tree cache miss after single-flight: {key}"))
}

/// Cached, recursive tree file paths (mirrors getGitHubTreePaths). `tree_sha`
/// is content-addressed, so the cache key is the SHA itself and entries never
/// expire — only the branch-alias refreshes above burn API budget.
pub async fn get_github_tree_paths(storage: &SharedStorage, tree_sha: &str) -> Result<Vec<String>> {
    let cache_key = format!(
        "{}/tree-paths/{}.json",
        crate::winget::constants::cache_prefix(),
        tree_sha
    );
    let tree_sha = tree_sha.to_string();
    cached_immutable(storage, &cache_key, async move {
        let tree = get_github_tree(&tree_sha, true).await?;
        Ok(tree.tree.into_iter().map(|i| i.path).collect())
    })
    .await
}

/// Cached letter-directory SHAs (a-z, 0-9) under manifests/ (mirrors getLetterDirectoryShas).
pub async fn get_letter_directory_shas(storage: &SharedStorage) -> Result<HashMap<String, String>> {
    // Resolve the branch alias first (TTL'd), then cache the letter map under
    // the immutable manifests SHA — as long as manifests/ doesn't change, the
    // letter tree is never re-requested.
    let manifests_sha = fetch_manifests_sha(storage).await?;
    let cache_key = format!(
        "{}/letter-shas/{}.json",
        crate::winget::constants::cache_prefix(),
        manifests_sha
    );
    cached_immutable(storage, &cache_key, async move {
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
    cached_singleflight(
        storage,
        &crate::winget::constants::manifests_sha_key(),
        async {
            let root = get_github_tree(crate::config::winget_github_branch(), false).await?;
            let manifests = root
                .tree
                .iter()
                .find(|i| i.path == "manifests" && i.item_type == "tree")
                .ok_or_else(|| anyhow::anyhow!("manifests directory not found in repository"))?;
            Ok(manifests.sha.clone())
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CacheMeta;
    use crate::storage::fs::FsStorage;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn stale_tree_is_served_while_background_refresh_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let storage: SharedStorage = Arc::new(FsStorage::new(tmp.path().to_str().unwrap()));
        let key = format!("{}/test-stale", crate::winget::constants::cache_prefix());
        let _ = storage.set_raw(&key, br#""old""#).await;
        let _ = storage
            .set_meta(
                &key,
                &CacheMeta {
                    mtime: Some((chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339()),
                    ..Default::default()
                },
            )
            .await;

        let count = Arc::new(AtomicUsize::new(0));
        let value = cached_singleflight(&storage, &key, {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                Ok("new".to_string())
            }
        })
        .await
        .unwrap();

        assert_eq!(value, "old");
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let refreshed = storage.get_raw(&key).await.unwrap();
        assert_eq!(refreshed, br#""new""#.to_vec());
    }
}
