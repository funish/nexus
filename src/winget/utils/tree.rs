//! GitHub Trees API access with TTL caching (mirrors winget/tree.ts).
//!
//! Discovers manifest file paths under `manifests/<letter>/...` and caches the
//! letter-directory SHAs and tree paths with a 10-minute TTL.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use crate::storage::SharedStorage;

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

/// Fetch a GitHub tree by SHA or branch (mirrors getGitHubTree).
async fn get_github_tree(tree_sha: &str, recursive: bool) -> Result<TreeResponse> {
    let url = format!(
        "{WINGET_GITHUB_API_BASE}/repos/{WINGET_GITHUB_REPO}/git/trees/{tree_sha}{}",
        if recursive { "?recursive=1" } else { "" }
    );
    let mut req = super::http::HTTP_CLIENT
        .get(&url)
        .timeout(Duration::from_secs(30));
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch GitHub tree: {}", resp.status());
    }
    Ok(resp.json().await?)
}

/// Cached, recursive tree file paths (mirrors getGitHubTreePaths).
pub async fn get_github_tree_paths(
    storage: &SharedStorage,
    tree_sha: &str,
    cache_suffix: &str,
) -> Result<Vec<String>> {
    let normalized = cache_suffix.replace('/', "-");
    let cache_key = format!("{WINGET_CACHE_PREFIX}/{normalized}");

    if cache_fresh(storage, &cache_key).await
        && let Some(data) = storage.get_raw(&cache_key).await
        && let Ok(paths) = serde_json::from_slice::<Vec<String>>(&data)
    {
        return Ok(paths);
    }

    let tree = get_github_tree(tree_sha, true).await?;
    let paths: Vec<String> = tree.tree.into_iter().map(|i| i.path).collect();

    storage
        .set_raw(&cache_key, &serde_json::to_vec(&paths)?)
        .await;
    set_mtime(storage, &cache_key).await;
    Ok(paths)
}

/// Cached letter-directory SHAs (a-z, 0-9) under manifests/ (mirrors getLetterDirectoryShas).
pub async fn get_letter_directory_shas(storage: &SharedStorage) -> Result<HashMap<String, String>> {
    let cache_key = format!("{WINGET_CACHE_PREFIX}/letter-shas.json");

    if cache_fresh(storage, &cache_key).await
        && let Some(data) = storage.get_raw(&cache_key).await
        && let Ok(map) = serde_json::from_slice::<HashMap<String, String>>(&data)
    {
        return Ok(map);
    }

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

    storage
        .set_raw(&cache_key, &serde_json::to_vec(&shas)?)
        .await;
    set_mtime(storage, &cache_key).await;
    Ok(shas)
}

/// Cached SHA of the manifests/ directory (mirrors fetchManifestsSha).
pub async fn fetch_manifests_sha(storage: &SharedStorage) -> Result<String> {
    if cache_fresh(storage, WINGET_MANIFESTS_SHA_KEY).await
        && let Some(data) = storage.get_raw(WINGET_MANIFESTS_SHA_KEY).await
        && let Ok(s) = String::from_utf8(data)
    {
        return Ok(s);
    }

    let root = get_github_tree(WINGET_GITHUB_BRANCH, false).await?;
    let manifests = root
        .tree
        .iter()
        .find(|i| i.path == "manifests" && i.item_type == "tree")
        .ok_or_else(|| anyhow::anyhow!("manifests directory not found in repository"))?;

    let sha = manifests.sha.clone();
    storage
        .set_raw(WINGET_MANIFESTS_SHA_KEY, sha.as_bytes())
        .await;
    set_mtime(storage, WINGET_MANIFESTS_SHA_KEY).await;
    Ok(sha)
}

/// Whether a cache entry is present and younger than the update interval.
async fn cache_fresh(storage: &SharedStorage, key: &str) -> bool {
    let Some(meta) = storage.get_meta(key).await else {
        return false;
    };
    let Some(mtime) = meta.mtime else {
        return false;
    };
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&mtime) else {
        return false;
    };
    let age = chrono::Utc::now()
        .signed_duration_since(ts.with_timezone(&chrono::Utc))
        .num_seconds();
    age < WINGET_UPDATE_INTERVAL_SECS
}

/// Stamp a cache entry's mtime to "now".
async fn set_mtime(storage: &SharedStorage, key: &str) {
    storage
        .set_meta(
            key,
            &crate::storage::CacheMeta {
                mtime: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await;
}
