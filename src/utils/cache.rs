//! TTL cache helpers built on the storage layer (mtime-based expiry).

use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::storage::{CacheMeta, SharedStorage};

/// TTL for registry metadata such as GitHub version/tag lists. Matches jsDelivr's
/// version-list caching upper bound of 10 minutes (jsdelivr/jsdelivr#18376).
pub const META_CACHE_TTL_SECS: i64 = 600;

/// Whether `key` is present and younger than `ttl_secs`.
pub async fn cache_fresh(storage: &SharedStorage, key: &str, ttl_secs: i64) -> bool {
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
    age < ttl_secs
}

/// Stamp `key`'s mtime to now (mark the entry fresh).
pub async fn set_mtime(storage: &SharedStorage, key: &str) {
    if let Err(e) = storage
        .set_meta(
            key,
            &CacheMeta {
                mtime: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await
    {
        tracing::warn!("Failed to stamp mtime for {key}: {e}");
    }
}

/// Refresh a cached JSON value through single-flight. The leader performs the
/// upstream fetch and cache write; followers wait and then re-read storage.
async fn refresh_cached_json<T, F>(storage: SharedStorage, key: String, ttl_secs: i64, fetch: F)
where
    T: Serialize + DeserializeOwned + Send,
    F: std::future::Future<Output = Result<T>> + Send,
{
    let run_key = key.clone();
    crate::utils::singleflight::run_once(&run_key, move || async move {
        // Another waiting caller may have refreshed the entry while this one was
        // acquiring leadership.
        if cache_fresh(&storage, &key, ttl_secs).await {
            return;
        }
        // Acquire a download slot only on a cache miss — the real network fetch
        // happens here, and this single gate covers every registry metadata call
        // (npm/jsr/gh/cdnjs/org) routed through cached_json.
        let _permit = super::concurrency::DOWNLOAD_SEMAPHORE
            .acquire()
            .await
            .unwrap();
        if let Ok(v) = fetch.await
            && let Ok(bytes) = serde_json::to_vec(&v)
        {
            // Only stamp freshness after the data write succeeded — a fresh
            // mtime over a missing body would serve 10 minutes of empty hits.
            match storage.set_raw(&key, &bytes).await {
                Ok(()) => set_mtime(&storage, &key).await,
                Err(e) => tracing::warn!("Failed to refresh registry cache {key}: {e}"),
            }
        }
    })
    .await;
}

/// Fetch a JSON value through a TTL cache with stale-while-revalidate: return a
/// fresh value directly, or a stale value immediately while refreshing it in the
/// background. A complete miss blocks on a single-flight fetch. Successful
/// results are cached with an mtime; failures never overwrite existing data.
pub async fn cached_json<T, F>(
    storage: &SharedStorage,
    key: &str,
    ttl_secs: i64,
    fetch: F,
) -> Result<T>
where
    T: Serialize + DeserializeOwned + Send + 'static,
    F: std::future::Future<Output = Result<T>> + Send + 'static,
{
    if cache_fresh(storage, key, ttl_secs).await
        && let Some(data) = storage.get_raw(key).await
        && let Ok(v) = serde_json::from_slice::<T>(&data)
    {
        return Ok(v);
    }

    if let Some(data) = storage.get_raw(key).await
        && let Ok(stale) = serde_json::from_slice::<T>(&data)
    {
        // Skip the detached refresh when one is already in flight; spawning
        // another follower task per request would only pile up waiters under
        // a hot key while the leader is slow.
        if !crate::utils::singleflight::is_pending(key) {
            let refresh = refresh_cached_json(storage.clone(), key.to_string(), ttl_secs, fetch);
            tokio::spawn(refresh);
        }
        return Ok(stale);
    }

    refresh_cached_json(storage.clone(), key.to_string(), ttl_secs, fetch).await;
    storage
        .get_raw(key)
        .await
        .and_then(|data| serde_json::from_slice::<T>(&data).ok())
        .ok_or_else(|| anyhow::anyhow!("registry cache miss after single-flight: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::fs::FsStorage;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn stale_json_is_served_while_background_refresh_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let storage: SharedStorage = Arc::new(FsStorage::new(tmp.path().to_str().unwrap()));
        let key = "test-stale-json";
        let _ = storage.set_raw(key, br#""old""#).await;
        let _ = storage
            .set_meta(
                key,
                &CacheMeta {
                    mtime: Some((chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339()),
                    ..Default::default()
                },
            )
            .await;

        let count = Arc::new(AtomicUsize::new(0));
        let value = cached_json(&storage, key, META_CACHE_TTL_SECS, {
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
        let refreshed = storage.get_raw(key).await.unwrap();
        assert_eq!(refreshed, br#""new""#.to_vec());
    }
}
