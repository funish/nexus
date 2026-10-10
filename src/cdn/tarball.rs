use anyhow::Result;
use flate2::read::GzDecoder;
use futures::StreamExt;
use std::collections::HashSet;
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tar::Archive;
use tracing::{error, warn};

use super::constants::*;
use super::integrity::calculate_integrity;
use crate::storage::{CacheMeta, CdnFileMeta, SharedStorage};

/// Deduplication set for concurrent background-cache jobs (mirrors pendingTarballs).
static PENDING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// RAII guard that removes the cache_base from PENDING on drop, ensuring cleanup
/// across all return paths (including early returns and panics).
struct PendingGuard {
    key: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        PENDING.lock().unwrap().remove(&self.key);
    }
}

pub struct TarEntry {
    pub name: String,
    pub data: Vec<u8>,
}

pub fn extract_tgz(data: &[u8]) -> Result<Vec<TarEntry>> {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        // Skip directory entries: they hold no data. On filesystem backends a
        // cached "dist"/"src" empty file shadows its child paths (a "dist" file
        // blocks writing "dist/d3-selection.js"), silently dropping every file
        // under that directory. Only regular files are cached.
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = entry.path()?.to_string_lossy().to_string();
        let size = entry.size();
        let mut buf = Vec::with_capacity(size as usize);
        entry.read_to_end(&mut buf)?;
        entries.push(TarEntry {
            name: path,
            data: buf,
        });
    }

    Ok(entries)
}

pub fn extract_file_from_tgz(data: &[u8], filepath: &str) -> Option<Vec<u8>> {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    // Single pass over the gzip stream: derive the root dir from the first real
    // (non-pax) entry, then match "{root}/{filepath}" as entries stream by. This
    // avoids decoding the gzip stream twice (a separate detect_root_dir scan plus
    // a second pass to match the file).
    let mut root: Option<String> = None;
    for entry in archive.entries().ok()? {
        let Ok(mut entry) = entry else { continue };
        let Ok(path) = entry.path() else { continue };
        let name = path.to_string_lossy();
        if root.is_none() && !name.starts_with("pax_global_header") {
            root = Some(name.split('/').next().unwrap_or("package").to_string());
        }
        let Some(root) = root.as_deref() else {
            continue;
        };
        if name == format!("{root}/{filepath}") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).ok()?;
            return Some(buf);
        }
    }

    None
}

pub async fn download_tarball(url: &str) -> Result<Vec<u8>, crate::utils::http::DownloadError> {
    // Each fetch materializes up to CDN_MAX_PACKAGE_SIZE in memory before
    // extraction; this bounds the cold-start memory peak rather than npm traffic.
    let _permit = crate::utils::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .unwrap();

    // download_to_vec retries retryable statuses AND mid-body transport
    // failures from scratch, so a 50MB tarball that drops mid-stream gets a
    // second full attempt instead of failing the request.
    crate::utils::http::download_to_vec(
        url,
        Duration::from_secs(CDN_FETCH_TIMEOUT_SECS),
        CDN_MAX_PACKAGE_SIZE,
    )
    .await
}

pub async fn extract_file_from_tarball(
    storage: &SharedStorage,
    tarball_url: &str,
    filepath: &str,
    cache_key: &str,
    direct_url: Option<&str>,
    warm: Option<(&str, &str)>,
    ttl_secs: Option<i64>,
) -> Result<Vec<u8>> {
    if cached_entry_valid(storage, cache_key, ttl_secs).await
        && let Some(cached) = storage.get_raw(cache_key).await
    {
        return Ok(cached);
    }

    // Single-flight: dedup concurrent cache-miss for the same key so only the
    // leader downloads; followers wait, then re-read storage.
    let storage_for_fn = storage.clone();
    let warm = warm.map(|(base, label)| (base.to_string(), label.to_string()));
    crate::utils::singleflight::run_once(cache_key, || {
        let storage = storage_for_fn.clone();
        let tarball_url = tarball_url.to_string();
        let filepath = filepath.to_string();
        let direct_url = direct_url.map(|u| u.to_string());
        let cache_key = cache_key.to_string();
        let warm = warm.clone();
        let ttl_secs_clone = ttl_secs;
        async move {
            // Re-check: another leader may have just cached it.
            if cached_entry_valid(&storage, &cache_key, ttl_secs_clone).await {
                return;
            }
            if let Some(url) = direct_url.as_deref() {
                if let Some(data) = try_fetch(url).await {
                    if storage.set_raw(&cache_key, &data).await.is_ok() {
                        crate::utils::cache::set_mtime(&storage, &cache_key).await;
                    } else {
                        warn!("Failed to cache {cache_key}");
                    }
                    return;
                }
                if url.contains("/main/") {
                    let master_url = url.replace("/main/", "/master/");
                    if let Some(data) = try_fetch(&master_url).await {
                        if storage.set_raw(&cache_key, &data).await.is_ok() {
                            crate::utils::cache::set_mtime(&storage, &cache_key).await;
                        } else {
                            warn!("Failed to cache {cache_key}");
                        }
                        return;
                    }
                    if let Ok(mut url_obj) = url::Url::parse(&url.replace(
                        "https://raw.githubusercontent.com/",
                        "https://cdn.jsdelivr.net/gh/",
                    )) {
                        let mut parts: Vec<&str> = url_obj.path().split('/').collect();
                        if parts.len() >= 6 {
                            parts.remove(4);
                        }
                        url_obj.set_path(&parts.join("/"));
                        if let Some(data) = try_fetch(url_obj.as_str()).await {
                            if storage.set_raw(&cache_key, &data).await.is_ok() {
                                crate::utils::cache::set_mtime(&storage, &cache_key).await;
                            } else {
                                warn!("Failed to cache {cache_key}");
                            }
                            return;
                        }
                    }
                }
            }
            match download_tarball(&tarball_url).await {
                Ok(tarball) => {
                    // Gzip inflate scans the whole archive; offload to the blocking
                    // pool so the async worker stays responsive during the scan.
                    let filepath = filepath.clone();
                    let tarball_for_extract = tarball.clone();
                    let extracted = tokio::task::spawn_blocking(move || {
                        extract_file_from_tgz(&tarball_for_extract, &filepath)
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some(data) = extracted {
                        if storage.set_raw(&cache_key, &data).await.is_ok() {
                            crate::utils::cache::set_mtime(&storage, &cache_key).await;
                        } else {
                            warn!("Failed to cache {cache_key}");
                        }
                    }
                    // Warm the full package in the background reusing these bytes, so a
                    // follow-up request for another file (or a directory listing) is served
                    // from cache without re-downloading the tarball. Only the download path
                    // has bytes to reuse; the direct_url fast path above never fetched the
                    // tarball, so gh (which uses direct_url) warms via its own spawn.
                    if let Some((base, label)) = warm {
                        let storage = storage.clone();
                        tokio::spawn(async move {
                            let _ =
                                cache_package_from_bytes(&storage, tarball, &base, &label).await;
                        });
                    }
                }
                // Transient upstream faults must not be silently mapped to a
                // not-found below; log them so misrouted 404s are diagnosable.
                Err(e) => {
                    warn!("Tarball fallback failed for {cache_key}: {e}");
                }
            }
        }
    })
    .await;

    storage
        .get_raw(cache_key)
        .await
        .ok_or_else(|| anyhow::anyhow!("File not found: {filepath}"))
}

/// Returns the cached package meta (with its file list) when the exact-version
/// cache entry exists, else `None`. Storage presence is independent of the HTTP
/// cache policy: a `latest`/range alias resolving to a cached exact version is a
/// storage hit even though the response must use the short alias Cache-Control.
/// Callers reuse `files[].integrity` as an ETag to avoid re-hashing the body.
pub async fn is_package_cached(storage: &SharedStorage, cache_base: &str) -> Option<CacheMeta> {
    let meta = storage.get_meta(cache_base).await?;
    (meta.files.is_some()).then_some(meta)
}

/// Outcome of attempting to claim a package cache job.
enum CacheSlot {
    /// This caller owns the job; the guard releases PENDING on drop.
    Owned(PendingGuard),
    /// Already cached (files present) or recently skipped — nothing to do.
    Cached,
    /// Another task is currently caching this package.
    Busy,
}

/// Skip if already cached (files present) or recently skipped; otherwise claim the
/// PENDING slot to dedup concurrent cache jobs.
async fn try_acquire_cache_slot(storage: &SharedStorage, cache_base: &str) -> CacheSlot {
    if let Some(meta) = storage.get_meta(cache_base).await {
        if meta.files.is_some() {
            return CacheSlot::Cached;
        }
        if let Some(skipped) = meta.skipped_at {
            let now = now_millis();
            if now - skipped < CDN_SKIP_TTL_MS {
                return CacheSlot::Cached;
            }
        }
    }

    let mut set = PENDING.lock().unwrap();
    if set.contains(cache_base) {
        return CacheSlot::Busy;
    }
    set.insert(cache_base.to_string());
    CacheSlot::Owned(PendingGuard {
        key: cache_base.to_string(),
    })
}

/// Whether a cached raw entry may be used. `ttl_secs` applies the mtime-based
/// freshness check (mutable refs such as branches/tags); `None` trusts presence.
async fn cached_entry_valid(
    storage: &SharedStorage,
    cache_key: &str,
    ttl_secs: Option<i64>,
) -> bool {
    match ttl_secs {
        Some(ttl) => crate::utils::cache::cache_fresh(storage, cache_key, ttl).await,
        None => storage.get_raw(cache_key).await.is_some(),
    }
}

/// Extract every file from an already-obtained tarball into storage and write the
/// file-list meta. Shared by the download entry point and the byte-reuse warm path.
/// The caller must hold the PENDING slot (via `try_acquire_cache_slot`) so concurrent
/// jobs for the same package don't duplicate the extract.
async fn cache_package_entries(
    storage: &SharedStorage,
    tarball_data: &[u8],
    cache_base: &str,
    log_label: &str,
) -> Result<()> {
    // Extraction, root derivation, and SRI hashing are all CPU-bound; run them
    // as one blocking-pool job so async workers stay free during the whole
    // compute phase instead of stalling on per-file SHA-256 calls.
    let data = tarball_data.to_vec();
    let cache_base_owned = cache_base.to_string();
    let (files, file_list): (Vec<(String, Vec<u8>)>, Vec<CdnFileMeta>) =
        tokio::task::spawn_blocking(move || {
            let entries = extract_tgz(&data)?;
            // Derive root from the first real entry (pax_global_header is tar
            // metadata, not a package path), then keep only entries under it.
            let root_dir = entries
                .iter()
                .find(|e| !e.name.starts_with("pax_global_header"))
                .map(|e| e.name.split('/').next().unwrap_or("package").to_string())
                .unwrap_or_else(|| "package".to_string());
            let root_path = format!("{root_dir}/");
            let mut files = Vec::new();
            let mut file_list = Vec::new();
            for entry in &entries {
                if !entry.name.starts_with(&root_path) {
                    continue;
                }
                let relative = entry.name[root_path.len()..].to_string();
                let integrity = calculate_integrity(&entry.data);
                files.push((
                    format!("{}/{relative}", cache_base_owned),
                    entry.data.clone(),
                ));
                file_list.push(CdnFileMeta {
                    name: relative,
                    size: entry.data.len() as u64,
                    integrity: Some(integrity),
                });
            }
            anyhow::Ok((files, file_list))
        })
        .await
        .map_err(|e| anyhow::anyhow!("tarball extract task failed: {e}"))??;

    let total_size: u64 = file_list.iter().map(|f| f.size).sum();
    if total_size > CDN_MAX_PACKAGE_SIZE {
        warn!(
            "Skipping {log_label}: unpacked size {} MB exceeds {} MB limit",
            total_size / 1024 / 1024,
            CDN_MAX_PACKAGE_SIZE / 1024 / 1024
        );
        storage
            .set_meta(
                cache_base,
                &CacheMeta {
                    skipped_at: Some(now_millis()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to record skip marker for {cache_base}: {e}"))?;
        return Ok(());
    }

    // PENDING slot guarantees no concurrent job caches the same package, so every
    // key here is fresh — write directly without a per-file get_raw pre-check.
    // Writes are pure local IO (fs / S3 PUT), so a bounded parallel width avoids
    // serializing hundreds of round-trips without stressing any upstream.
    let mut writes = futures::stream::iter(files)
        .map(|(key, data)| {
            let storage = storage.clone();
            async move { storage.set_raw(&key, &data).await.map(|_| key) }
        })
        .buffered(*crate::utils::concurrency::STORAGE_WRITE_CONCURRENCY);
    while let Some(result) = writes.next().await {
        result
            .map_err(|e| anyhow::anyhow!("failed to cache package files for {cache_base}: {e}"))?;
    }

    storage
        .set_meta(
            cache_base,
            &CacheMeta {
                files: Some(file_list),
                ..Default::default()
            },
        )
        .await?;

    Ok(())
}

pub async fn cache_package_from_tarball(
    storage: &SharedStorage,
    tarball_url: &str,
    cache_base: &str,
    log_label: &str,
) -> Result<()> {
    let _guard = match await_cache_slot(storage, cache_base).await {
        Some(g) => g,
        None => return Ok(()),
    };

    let tarball_data = match download_tarball(tarball_url).await {
        Ok(data) => data,
        // Deterministic misses get the short skipped_at negative cache; the
        // empty listing stays a 200 for compatibility.
        Err(
            e @ (crate::utils::http::DownloadError::NotFound
            | crate::utils::http::DownloadError::TooLarge),
        ) => {
            warn!("Tarball unavailable for {log_label}: {e}");
            storage
                .set_meta(
                    cache_base,
                    &CacheMeta {
                        skipped_at: Some(now_millis()),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!("failed to record skip marker for {cache_base}: {e}")
                })?;
            return Ok(());
        }
        // Transient upstream faults (429/5xx/transport) must not be frozen into
        // a negative cache — surface them so listings return 502 and the next
        // request retries.
        Err(e) => {
            error!("Failed to download tarball for {log_label}: {e}");
            return Err(anyhow::anyhow!("{e}"));
        }
    };

    cache_package_entries(storage, &tarball_data, cache_base, log_label).await
}

/// Cache a full package from tarball bytes the caller already downloaded (e.g. the
/// foreground sub-path request that extracted one file). Avoids a second tarball
/// download for the background warm path. Same skip/dedup semantics as
/// `cache_package_from_tarball`.
pub async fn cache_package_from_bytes(
    storage: &SharedStorage,
    tarball_data: Vec<u8>,
    cache_base: &str,
    log_label: &str,
) -> Result<()> {
    let _guard = match await_cache_slot(storage, cache_base).await {
        Some(g) => g,
        None => return Ok(()),
    };

    cache_package_entries(storage, &tarball_data, cache_base, log_label).await
}

/// Claim a package cache slot, waiting out a concurrent job instead of skipping.
/// Previously, concurrent cold requests saw `Busy` and returned immediately,
/// reading an empty listing or failing `+esm` before the leader had written the
/// meta. Waits up to two minutes (a 50MB tarball download on a slow link), then
/// degrades to the old skip behavior so a stuck leader cannot hold requests.
async fn await_cache_slot(storage: &SharedStorage, cache_base: &str) -> Option<PendingGuard> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match try_acquire_cache_slot(storage, cache_base).await {
            CacheSlot::Owned(guard) => return Some(guard),
            CacheSlot::Cached => return None,
            CacheSlot::Busy => {
                if tokio::time::Instant::now() >= deadline {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

pub async fn try_fetch(url: &str) -> Option<Vec<u8>> {
    let _permit = crate::utils::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .ok()?;
    // Same streaming size cap and mid-body retry as download_tarball: wp zips
    // and gh raw files flow through here and either can be oversized or drop
    // mid-transfer. Fail-open to the caller's fallback URL on any error.
    crate::utils::http::download_to_vec(
        url,
        Duration::from_secs(CDN_FETCH_TIMEOUT_SECS),
        CDN_MAX_PACKAGE_SIZE,
    )
    .await
    .ok()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
