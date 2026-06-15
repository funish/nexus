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
    storage
        .set_meta(
            key,
            &CacheMeta {
                mtime: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await;
}

/// Fetch a JSON value through a TTL cache: return the cached value when fresh,
/// otherwise run `fetch`, cache a successful result (mtime stamped), and return
/// it. Failures are never cached, so a source outage can't pin a stale/empty result.
pub async fn cached_json<T>(
    storage: &SharedStorage,
    key: &str,
    ttl_secs: i64,
    fetch: impl std::future::Future<Output = Result<T>>,
) -> Result<T>
where
    T: Serialize + DeserializeOwned,
{
    if cache_fresh(storage, key, ttl_secs).await
        && let Some(data) = storage.get_raw(key).await
        && let Ok(v) = serde_json::from_slice::<T>(&data)
    {
        return Ok(v);
    }
    let v = fetch.await?;
    if let Ok(bytes) = serde_json::to_vec(&v) {
        storage.set_raw(key, &bytes).await;
        set_mtime(storage, key).await;
    }
    Ok(v)
}
