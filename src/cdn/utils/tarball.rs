use anyhow::Result;
use flate2::read::GzDecoder;
use std::collections::{HashMap, HashSet};
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
    // Determine the root dir once with a dedicated scan, then match (mirrors tar.ts:
    // detectRootDirFromBytes first, then extractFileFromTgz matches against it).
    let root = detect_root_dir(data);
    let full_path = format!("{root}/{filepath}");

    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries().ok()? {
        let mut entry = entry.ok()?;
        let path = entry.path().ok()?;
        if path.to_string_lossy() == full_path {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).ok()?;
            return Some(buf);
        }
    }

    None
}

pub fn detect_root_dir(data: &[u8]) -> String {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries().into_iter().flatten() {
        if let Ok(entry) = entry
            && let Ok(path) = entry.path()
        {
            let path_str = path.to_string_lossy();
            if !path_str.starts_with("pax_global_header") {
                return path_str.split('/').next().unwrap_or("package").to_string();
            }
        }
    }

    "package".to_string()
}

pub async fn download_tarball(url: &str) -> Result<Vec<u8>> {
    // Cap concurrent outbound fetches so a burst of cache misses doesn't trip
    // npm's per-IP rate limit (429 / IP block).
    let _permit = super::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
        .build()?;

    let mut resp = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            anyhow::anyhow!("Tarball download timed out")
        } else {
            anyhow::anyhow!("Failed to download tarball: {e}")
        }
    })?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to download tarball: {}", resp.status());
    }

    // Reject before reading the body when the server declares an oversized
    // Content-Length, then stream with a hard cap so a missing/lying header
    // (or a huge gh repo tarball) can't exhaust memory or bandwidth. The
    // post-extract CDN_MAX_PACKAGE_SIZE check in cache_package_from_tarball
    // still guards the *unpacked* size (tarballs compress).
    if let Some(len) = resp.content_length()
        && len > CDN_MAX_PACKAGE_SIZE
    {
        anyhow::bail!(
            "Tarball exceeds {} byte limit (declared {len})",
            CDN_MAX_PACKAGE_SIZE
        );
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > CDN_MAX_PACKAGE_SIZE as usize {
            anyhow::bail!(
                "Tarball exceeded {} byte limit while streaming",
                CDN_MAX_PACKAGE_SIZE
            );
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn extract_file_from_tarball(
    storage: &SharedStorage,
    tarball_url: &str,
    filepath: &str,
    cache_key: &str,
    direct_url: Option<&str>,
) -> Result<Vec<u8>> {
    if let Some(cached) = storage.get_raw(cache_key).await {
        return Ok(cached);
    }

    // Single-flight: dedup concurrent cache-miss for the same key so only the
    // leader downloads; followers wait, then re-read storage.
    let storage_for_fn = storage.clone();
    super::singleflight::run_once(cache_key, || {
        let storage = storage_for_fn.clone();
        let tarball_url = tarball_url.to_string();
        let filepath = filepath.to_string();
        let direct_url = direct_url.map(|u| u.to_string());
        let cache_key = cache_key.to_string();
        async move {
            // Re-check: another leader may have just cached it.
            if storage.get_raw(&cache_key).await.is_some() {
                return;
            }
            if let Some(url) = direct_url.as_deref() {
                if let Some(data) = try_fetch(url).await {
                    storage.set_raw(&cache_key, &data).await;
                    return;
                }
                if url.contains("/main/") {
                    let master_url = url.replace("/main/", "/master/");
                    if let Some(data) = try_fetch(&master_url).await {
                        storage.set_raw(&cache_key, &data).await;
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
                            storage.set_raw(&cache_key, &data).await;
                            return;
                        }
                    }
                }
            }
            if let Ok(tarball) = download_tarball(&tarball_url).await
                && let Some(data) = extract_file_from_tgz(&tarball, &filepath)
            {
                storage.set_raw(&cache_key, &data).await;
            }
        }
    })
    .await;

    storage
        .get_raw(cache_key)
        .await
        .ok_or_else(|| anyhow::anyhow!("File not found: {filepath}"))
}

pub async fn is_package_cached(storage: &SharedStorage, cache_base: &str, cacheable: bool) -> bool {
    if !cacheable {
        return false;
    }
    storage
        .get_meta(cache_base)
        .await
        .and_then(|m| m.files)
        .is_some()
}

pub async fn cache_package_from_tarball(
    storage: &SharedStorage,
    tarball_url: &str,
    cache_base: &str,
    log_label: &str,
) -> Result<()> {
    if let Some(meta) = storage.get_meta(cache_base).await {
        if meta.files.is_some() {
            return Ok(());
        }
        if let Some(skipped) = meta.skipped_at {
            let now = now_millis();
            if now - skipped < CDN_SKIP_TTL_MS {
                return Ok(());
            }
        }
    }

    // Deduplicate concurrent cache jobs for the same package (mirrors pendingTarballs).
    let _guard = {
        let mut set = PENDING.lock().unwrap();
        if set.contains(cache_base) {
            return Ok(());
        }
        set.insert(cache_base.to_string());
        PendingGuard {
            key: cache_base.to_string(),
        }
    };

    let tarball_data = match download_tarball(tarball_url).await {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to download tarball for {log_label}: {e}");
            storage
                .set_meta(
                    cache_base,
                    &CacheMeta {
                        skipped_at: Some(now_millis()),
                        ..Default::default()
                    },
                )
                .await;
            return Ok(());
        }
    };

    let root_dir = detect_root_dir(&tarball_data);
    let root_path = format!("{root_dir}/");
    let entries = extract_tgz(&tarball_data)?;

    let filtered: Vec<&TarEntry> = entries
        .iter()
        .filter(|e| e.name.starts_with(&root_path))
        .collect();

    let mut file_list: Vec<CdnFileMeta> = filtered
        .iter()
        .map(|e| CdnFileMeta {
            name: e.name[root_path.len()..].to_string(),
            size: e.data.len() as u64,
            integrity: None,
        })
        .collect();

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
            .await;
        return Ok(());
    }

    let mut file_index: HashMap<String, usize> = HashMap::new();
    for (i, f) in file_list.iter().enumerate() {
        file_index.insert(f.name.clone(), i);
    }

    for entry in &filtered {
        let relative = &entry.name[root_path.len()..];
        let key = format!("{cache_base}/{relative}");

        if storage.get_raw(&key).await.is_some() {
            continue;
        }

        storage.set_raw(&key, &entry.data).await;

        let integrity = calculate_integrity(&entry.data);
        if let Some(&idx) = file_index.get(relative) {
            file_list[idx].integrity = Some(integrity);
        }
    }

    storage
        .set_meta(
            cache_base,
            &CacheMeta {
                files: Some(file_list),
                ..Default::default()
            },
        )
        .await;

    Ok(())
}

pub async fn try_fetch(url: &str) -> Option<Vec<u8>> {
    let _permit = super::concurrency::DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .ok()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CDN_FETCH_TIMEOUT_SECS))
        .build()
        .ok()?;
    let mut resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    // Same streaming size cap as download_tarball: wp zips and gh raw files
    // flow through here and either can be oversized.
    if let Some(len) = resp.content_length()
        && len > CDN_MAX_PACKAGE_SIZE
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > CDN_MAX_PACKAGE_SIZE as usize {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
