use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::queries::build_search_index;
use super::search::WinGetSearchEntry;
use crate::storage::{CacheMeta, SharedStorage};

const WINGET_SEARCH_INDEX_KEY: &str = "winget/index.json";
const WINGET_DB_UPDATE_INTERVAL_SECS: u64 = 900;

/// Bind cached database bytes and refresh single-flight to the exact upstream, so
/// different configured sources never reuse each other's metadata.
fn index_db_key() -> String {
    let source_hash = sha256_hex(crate::config::winget_source_msix_url().as_bytes());
    format!("winget/index-db/{source_hash}/index.db")
}

fn index_db_load_key() -> String {
    format!("{}/load", index_db_key())
}

fn index_db_refresh_key() -> String {
    format!("{}/refresh", index_db_key())
}

pub type SharedDb = Arc<Mutex<Option<CachedDb>>>;

/// Number of read-only SQLite connections in the pool. Concurrent reads on a
/// read-only database are safe; multiple connections remove the single-mutex
/// bottleneck for per-request `package_exists` / version queries. Override via
/// `NEXUS_DB_POOL_SIZE`.
fn read_pool_size() -> usize {
    std::env::var("NEXUS_DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 1)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(4)
        })
}

/// A cloneable handle to a pool of read-only connections over one immutable
/// SQLite snapshot. `lock()` round-robins across the pool so concurrent
/// requests don't serialize on a single mutex. Dropping the final handle closes
/// all connections before [`SnapshotFile`] removes the backing file.
#[derive(Clone)]
pub struct Database {
    connections: Arc<Vec<Mutex<Connection>>>,
    next: Arc<AtomicUsize>,
    _snapshot: Arc<SnapshotFile>,
}

struct SnapshotFile {
    path: PathBuf,
}

impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Database {
    /// Acquire a read-only connection, distributing across the pool round-robin.
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        self.connections[idx]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone)]
pub struct CachedDb {
    pub database: Database,
    pub checked_at: u64,
    pub search_index: Arc<Vec<WinGetSearchEntry>>,
}

pub fn create_shared_db() -> SharedDb {
    Arc::new(Mutex::new(None))
}

/// Initialize a source database once, then expose it as read-only. Index creation
/// is deliberately not repeated on request paths.
/// Open one read-only connection over the snapshot.
fn open_read_connection(path: &PathBuf) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let _ = conn.busy_timeout(Duration::from_secs(30));
    Ok(conn)
}

/// Initialize a source database once, then expose it as a read-only pool.
/// Index creation is deliberately not repeated on request paths.
fn open_snapshot(path: &PathBuf, build_indexes: bool) -> Result<Vec<Connection>> {
    if build_indexes {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let _ = conn.busy_timeout(Duration::from_secs(30));
        conn.execute_batch(
            "PRAGMA temp_store = MEMORY;
             CREATE INDEX IF NOT EXISTS tags_map_manifest_idx ON tags_map(manifest);
             CREATE INDEX IF NOT EXISTS commands_map_manifest_idx ON commands_map(manifest);",
        )?;
        if let Err((conn, e)) = conn.close() {
            drop(conn);
            return Err(e.into());
        }
    }

    let pool_size = read_pool_size();
    let mut connections = Vec::with_capacity(pool_size);
    for _ in 0..pool_size {
        connections.push(open_read_connection(path)?);
    }
    Ok(connections)
}

fn now_secs() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn meta_string(meta: &CacheMeta, key: &str) -> Option<String> {
    meta.extra.get(key)?.as_str().map(String::from)
}

fn checked_at_from_meta(meta: &CacheMeta) -> u64 {
    meta_string(meta, "checked_at")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.with_timezone(&Utc).timestamp().max(0) as u64)
        .unwrap_or(0)
}

fn checked_meta(mut meta: CacheMeta, etag: Option<&str>, db_hash: Option<&str>) -> CacheMeta {
    let now = Utc::now().to_rfc3339();
    meta.mtime = Some(now.clone());
    meta.extra
        .insert("checked_at".to_string(), Value::String(now));
    if let Some(etag) = etag {
        meta.extra
            .insert("etag".to_string(), Value::String(etag.to_string()));
    }
    if let Some(db_hash) = db_hash {
        meta.extra
            .insert("db_sha256".to_string(), Value::String(db_hash.to_string()));
    }
    meta
}

fn current_cached(db: &SharedDb) -> Option<CachedDb> {
    let guard = db.lock().unwrap_or_else(PoisonError::into_inner);
    guard.clone()
}

fn fresh_cached(db: &SharedDb) -> Option<CachedDb> {
    let cached = current_cached(db)?;
    let now = now_secs().ok()?;
    (now.saturating_sub(cached.checked_at) < WINGET_DB_UPDATE_INTERVAL_SECS).then_some(cached)
}

fn store_cached(db: &SharedDb, cached: CachedDb) {
    let mut guard = db.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(cached);
}

/// Write an immutable unique snapshot. Old handles continue using their own file;
/// cleanup happens automatically when the final connection handle is dropped.
fn create_database(data: &[u8], build_indexes: bool) -> Result<Database> {
    let temp = tempfile::NamedTempFile::new()?;
    temp.as_file().write_all(data)?;
    let path = temp.into_temp_path().keep()?;
    let snapshot = Arc::new(SnapshotFile { path: path.clone() });

    let connections = open_snapshot(&path, build_indexes)?;
    Ok(Database {
        connections: Arc::new(connections.into_iter().map(Mutex::new).collect()),
        next: Arc::new(AtomicUsize::new(0)),
        _snapshot: snapshot,
    })
}

async fn prepare_database(data: Vec<u8>, build_indexes: bool) -> Result<Database> {
    tokio::task::spawn_blocking(move || create_database(&data, build_indexes)).await?
}

async fn load_persisted_index(
    storage: &SharedStorage,
    db_hash: &str,
) -> Result<Option<Arc<Vec<WinGetSearchEntry>>>> {
    let Some(meta) = storage.get_meta(WINGET_SEARCH_INDEX_KEY).await else {
        return Ok(None);
    };
    if meta_string(&meta, "db_sha256").as_deref() != Some(db_hash) {
        return Ok(None);
    }
    let Some(bytes) = storage.get_raw(WINGET_SEARCH_INDEX_KEY).await else {
        return Ok(None);
    };

    let parsed = tokio::task::spawn_blocking(move || {
        serde_json::from_slice::<Vec<WinGetSearchEntry>>(&bytes)
    })
    .await?;
    Ok(parsed.ok().map(Arc::new))
}

async fn build_and_persist_index(
    database: &Database,
    storage: &SharedStorage,
    db_hash: &str,
) -> Result<Arc<Vec<WinGetSearchEntry>>> {
    let blocking_database = database.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let conn = blocking_database.lock();
        build_search_index(&conn)
    })
    .await??;

    let entries = Arc::new(entries);
    let bytes = tokio::task::spawn_blocking({
        let entries = entries.clone();
        move || serde_json::to_vec(&*entries)
    })
    .await??;

    storage.set_raw(WINGET_SEARCH_INDEX_KEY, &bytes).await;
    storage
        .set_meta(
            WINGET_SEARCH_INDEX_KEY,
            &CacheMeta {
                mtime: Some(Utc::now().to_rfc3339()),
                extra: HashMap::from([(
                    "db_sha256".to_string(),
                    Value::String(db_hash.to_string()),
                )]),
                ..Default::default()
            },
        )
        .await;
    Ok(entries)
}

async fn load_index_db_from_storage(
    db: &SharedDb,
    storage: &SharedStorage,
) -> Result<Option<Database>> {
    let Some(data) = storage.get_raw(index_db_key().as_str()).await else {
        return Ok(None);
    };
    let meta = storage
        .get_meta(index_db_key().as_str())
        .await
        .unwrap_or_default();
    let etag = meta_string(&meta, "etag");

    // Existing deployments may not yet have a hash. Compute it once, then persist it
    // so later restarts can bind search index files to this exact DB snapshot.
    let (data, db_hash) = if let Some(hash) = meta_string(&meta, "db_sha256") {
        (data, hash)
    } else {
        let hashed = tokio::task::spawn_blocking(move || {
            let hash = sha256_hex(&data);
            (data, hash)
        })
        .await?;
        let mut next_meta = meta.clone();
        next_meta
            .extra
            .insert("db_sha256".to_string(), Value::String(hashed.1.clone()));
        if let Some(etag) = &etag {
            next_meta
                .extra
                .insert("etag".to_string(), Value::String(etag.clone()));
        }
        storage.set_meta(index_db_key().as_str(), &next_meta).await;
        hashed
    };

    let search_index = load_persisted_index(storage, &db_hash).await?;
    let database = prepare_database(data, search_index.is_none()).await?;
    let search_index = if let Some(index) = search_index {
        index
    } else {
        build_and_persist_index(&database, storage, &db_hash).await?
    };

    let cached = CachedDb {
        database: database.clone(),
        checked_at: checked_at_from_meta(&meta),
        search_index,
    };
    store_cached(db, cached);
    Ok(Some(database))
}

async fn load_cached_db(db: &SharedDb, storage: &SharedStorage) -> Option<Database> {
    if let Some(cached) = fresh_cached(db) {
        return Some(cached.database);
    }

    let state = db.clone();
    let storage = storage.clone();
    crate::utils::singleflight::run_once(&index_db_load_key(), move || async move {
        if fresh_cached(&state).is_some() {
            return;
        }
        if let Err(e) = load_index_db_from_storage(&state, &storage).await {
            tracing::warn!("failed to load cached WinGet index.db: {e:#}");
        }
    })
    .await;

    current_cached(db).map(|cached| cached.database)
}

async fn refresh_upstream_index(db: &SharedDb, storage: &SharedStorage) -> Result<RefreshOutcome> {
    let old_meta = storage
        .get_meta(index_db_key().as_str())
        .await
        .unwrap_or_default();
    let etag = meta_string(&old_meta, "etag");

    let mut headers: Vec<(&str, &str)> = Vec::new();
    let conditional_etag = etag.clone();
    if let Some(etag) = &conditional_etag {
        headers.push(("If-None-Match", etag.as_str()));
    }

    let resp = crate::utils::http::get_with_retry(
        crate::config::winget_source_msix_url(),
        Duration::from_secs(120),
        None,
        &headers,
    )
    .await?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        let meta = checked_meta(old_meta, etag.as_deref(), None);
        storage.set_meta(index_db_key().as_str(), &meta).await;
        if let Some(mut cached) = current_cached(db) {
            cached.checked_at = now_secs()?;
            store_cached(db, cached);
        }
        return Ok(RefreshOutcome::NotModified);
    }

    if !resp.status().is_success() {
        anyhow::bail!("Failed to download source.msix: {}", resp.status());
    }

    // Never reuse an old ETag when the fresh response does not provide one.
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let source_version = resp
        .headers()
        .get("x-ms-meta-sourceversion")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let bytes = resp.bytes().await?;
    let (data, db_hash) = tokio::task::spawn_blocking(move || {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))?;
        let mut file = archive.by_name("Public/index.db")?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        let hash = sha256_hex(&data);
        anyhow::Ok((data, hash))
    })
    .await??;

    let search_index = load_persisted_index(storage, &db_hash).await?;

    // Persist before moving `data` into the snapshot loader — avoids cloning the
    // multi-MB database just to hand one copy to each of storage and SQLite.
    storage.set_raw(index_db_key().as_str(), &data).await;
    let database = prepare_database(data, search_index.is_none()).await?;
    let search_index = if let Some(index) = search_index {
        index
    } else {
        build_and_persist_index(&database, storage, &db_hash).await?
    };

    let mut meta = CacheMeta::default();
    if let Some(source_version) = source_version {
        meta.extra
            .insert("source_version".to_string(), Value::String(source_version));
    }
    let meta = checked_meta(meta, etag.as_deref(), Some(&db_hash));
    storage.set_meta(index_db_key().as_str(), &meta).await;

    let cached = CachedDb {
        database: database.clone(),
        checked_at: now_secs()?,
        search_index,
    };
    Ok(RefreshOutcome::Updated(cached))
}

enum RefreshOutcome {
    NotModified,
    Updated(CachedDb),
}

fn spawn_refresh_if_stale(db: &SharedDb, storage: &SharedStorage) {
    if fresh_cached(db).is_some() {
        return;
    }
    let state = db.clone();
    let refresh_storage = storage.clone();
    tokio::spawn(async move {
        let _ = refresh_index_db(&state, &refresh_storage).await;
    });
}

async fn refresh_index_db(db: &SharedDb, storage: &SharedStorage) -> Result<Database> {
    let state = db.clone();
    let refresh_storage = storage.clone();
    crate::utils::singleflight::run_once(&index_db_refresh_key(), move || async move {
        if fresh_cached(&state).is_some() {
            return;
        }
        match refresh_upstream_index(&state, &refresh_storage).await {
            Ok(RefreshOutcome::Updated(cached)) => store_cached(&state, cached),
            Ok(RefreshOutcome::NotModified) => {}
            Err(e) => tracing::warn!("failed to refresh WinGet source.msix: {e:#}"),
        }
    })
    .await;

    current_cached(db)
        .map(|cached| cached.database)
        .ok_or_else(|| anyhow::anyhow!("WinGet index.db unavailable after refresh"))
}

/// Return a snapshot handle. A fresh snapshot is returned directly; an expired but
/// available snapshot is returned while one single-flight background refresh runs.
pub async fn get_index_db(db: &SharedDb, storage: &SharedStorage) -> Result<Database> {
    if let Some(cached) = fresh_cached(db) {
        return Ok(cached.database);
    }

    if let Some(cached) = current_cached(db) {
        spawn_refresh_if_stale(db, storage);
        return Ok(cached.database);
    }

    if let Some(database) = load_cached_db(db, storage).await {
        spawn_refresh_if_stale(db, storage);
        return Ok(database);
    }

    refresh_index_db(db, storage).await
}

/// Return the in-memory search index. It has no time TTL: a rebuilt index replaces
/// it only when the DB snapshot hash changes.
pub async fn get_search_index(
    db: &SharedDb,
    storage: &SharedStorage,
) -> Result<Arc<Vec<WinGetSearchEntry>>> {
    // Also starts the background refresh when the snapshot has gone stale.
    get_index_db(db, storage).await?;
    let guard = db.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .as_ref()
        .map(|cached| cached.search_index.clone())
        .ok_or_else(|| anyhow::anyhow!("search index missing after db load"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_removed_after_final_database_handle_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blank.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE tags_map(manifest INTEGER);
             CREATE TABLE commands_map(manifest INTEGER);",
        )
        .unwrap();
        drop(conn);

        let data = std::fs::read(&path).unwrap();
        let database = create_database(&data, true).unwrap();
        let raw_path = database._snapshot.path.clone();
        assert!(raw_path.exists());

        let clone = database.clone();
        drop(database);
        assert!(raw_path.exists());
        drop(clone);
        assert!(!raw_path.exists());
    }
}
