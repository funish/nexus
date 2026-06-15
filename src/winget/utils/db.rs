use anyhow::Result;
use rusqlite::Connection;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::queries::build_search_index;
use super::search::WinGetSearchEntry;
use crate::storage::SharedStorage;

const WINGET_SOURCE_MSIX_URL: &str = "https://cdn.winget.microsoft.com/cache/source.msix";
const WINGET_INDEX_DB_KEY: &str = "winget/index.db";
const WINGET_SEARCH_INDEX_KEY: &str = "winget/index.json";
const WINGET_UPDATE_INTERVAL_SECS: u64 = 600;

pub type SharedDb = Arc<Mutex<Option<CachedDb>>>;

pub struct CachedDb {
    pub db_path: String,
    pub loaded_at: u64,
    pub search_index: Option<Arc<Vec<WinGetSearchEntry>>>,
}

pub fn create_shared_db() -> SharedDb {
    Arc::new(Mutex::new(None))
}

pub fn open_db(path: &str) -> Result<Connection> {
    // Open read-write: the winget index.db ships without indexes on tags_map(manifest)
    // and commands_map(manifest), and build_search_index JOINs on those columns for every
    // manifest row. CREATE INDEX IF NOT EXISTS is idempotent, so this is a no-op after the
    // first open of a given temp copy.
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS tags_map_manifest_idx ON tags_map(manifest);
         CREATE INDEX IF NOT EXISTS commands_map_manifest_idx ON commands_map(manifest);",
    )?;
    Ok(conn)
}

fn now_secs() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

pub async fn get_index_db(db: &SharedDb, storage: &SharedStorage) -> Result<Connection> {
    // Check in-memory cache. Clone the path under the lock, then open outside it —
    // open_db does blocking sqlite IO and must not run while holding the mutex.
    let cached_path = {
        let guard = db.lock().unwrap();
        let now = now_secs()?;
        match &*guard {
            Some(cached) if now - cached.loaded_at < WINGET_UPDATE_INTERVAL_SECS => {
                Some(cached.db_path.clone())
            }
            _ => None,
        }
    };
    if let Some(path) = cached_path {
        return open_db(&path);
    }

    // Try loading from storage.
    if let Some(data) = storage.get_raw(WINGET_INDEX_DB_KEY).await {
        let path = write_to_temp(&data)?;
        let conn = open_db(&path)?;
        // Reuse the persisted index when still valid; rebuild only on miss so a
        // restart doesn't always pay the ~7s build when index.db is unchanged.
        let index = match load_persisted_index(storage).await {
            Some(idx) => idx,
            None => {
                let idx = Arc::new(build_search_index(&conn)?);
                persist_search_index(storage, &idx).await;
                idx
            }
        };
        let now = now_secs()?;
        {
            let mut guard = db.lock().unwrap();
            *guard = Some(CachedDb {
                db_path: path,
                loaded_at: now,
                search_index: Some(index),
            });
        }
        return Ok(conn);
    }

    // Download fresh.
    refresh_index_db(db, storage).await
}

async fn refresh_index_db(db: &SharedDb, storage: &SharedStorage) -> Result<Connection> {
    let resp = super::http::HTTP_CLIENT
        .get(WINGET_SOURCE_MSIX_URL)
        .timeout(Duration::from_secs(120))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to download source.msix: {}", resp.status());
    }

    let bytes = resp.bytes().await?;

    // MSIX is a ZIP file.
    let reader = std::io::Cursor::new(bytes.to_vec());
    let mut archive = zip::ZipArchive::new(reader)?;

    let index_db = {
        let mut file = archive.by_name("Public/index.db")?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        buf
    };

    storage.set_raw(WINGET_INDEX_DB_KEY, &index_db).await;
    storage
        .set_meta(
            WINGET_INDEX_DB_KEY,
            &crate::storage::CacheMeta {
                mtime: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await;

    let path = write_to_temp(&index_db)?;
    let conn = open_db(&path)?;
    let index = Arc::new(build_search_index(&conn)?);
    persist_search_index(storage, &index).await;
    let now = now_secs()?;
    {
        let mut guard = db.lock().unwrap();
        *guard = Some(CachedDb {
            db_path: path,
            loaded_at: now,
            search_index: Some(index),
        });
    }

    Ok(conn)
}

/// Get the search index (return on memory-cache hit; otherwise build it when the db loads).
pub async fn get_search_index(
    db: &SharedDb,
    storage: &SharedStorage,
) -> Result<Arc<Vec<WinGetSearchEntry>>> {
    // Fast path: cache hit and not expired.
    {
        let guard = db.lock().unwrap();
        if let Some(ref cached) = *guard {
            let now = now_secs()?;
            if now - cached.loaded_at < WINGET_UPDATE_INTERVAL_SECS
                && let Some(ref idx) = cached.search_index
            {
                return Ok(idx.clone());
            }
        }
    }

    // Persistent cache: avoid rebuild across restarts (mirrors getSearchIndex cacheStorage).
    if let Some(idx) = load_persisted_index(storage).await {
        return Ok(idx);
    }

    // Slow path: ensure the db is loaded (refresh/load both build the index).
    let _conn = get_index_db(db, storage).await?;
    {
        let guard = db.lock().unwrap();
        if let Some(ref cached) = *guard
            && let Some(ref idx) = cached.search_index
        {
            return Ok(idx.clone());
        }
    }

    // Fallback: build on the spot (refresh/load should have built it; defensive).
    let conn = {
        let guard = db.lock().unwrap();
        let path = guard.as_ref().expect("db should be loaded").db_path.clone();
        drop(guard);
        open_db(&path)?
    };
    let index = Arc::new(build_search_index(&conn)?);
    persist_search_index(storage, &index).await;
    {
        let mut guard = db.lock().unwrap();
        if let Some(ref mut cached) = *guard {
            cached.search_index = Some(index.clone());
        }
    }
    Ok(index)
}

fn write_to_temp(data: &[u8]) -> Result<String> {
    // Fixed path: overwritten each load/refresh, so no temp files accumulate.
    let path = std::env::temp_dir().join("nexus-winget-index.db");
    std::fs::write(&path, data)?;
    Ok(path.to_string_lossy().to_string())
}

/// Serialize the built search index to storage (mirrors search.ts persistSearchIndex).
async fn persist_search_index(storage: &SharedStorage, index: &[WinGetSearchEntry]) {
    let Ok(bytes) = serde_json::to_vec(index) else {
        return;
    };
    storage.set_raw(WINGET_SEARCH_INDEX_KEY, &bytes).await;
    storage
        .set_meta(
            WINGET_SEARCH_INDEX_KEY,
            &crate::storage::CacheMeta {
                mtime: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await;
}

/// Load the search index from storage if cached and not expired (mirrors
/// getSearchIndex cacheStorage fallback). None on miss/expiry/parse error → rebuild.
async fn load_persisted_index(storage: &SharedStorage) -> Option<Arc<Vec<WinGetSearchEntry>>> {
    let meta = storage.get_meta(WINGET_SEARCH_INDEX_KEY).await?;
    let mtime_str = meta.mtime.filter(|s| !s.is_empty())?;
    let mtime = chrono::DateTime::parse_from_rfc3339(&mtime_str).ok()?;
    let elapsed = chrono::Utc::now().signed_duration_since(mtime.with_timezone(&chrono::Utc));
    if elapsed.num_seconds() > WINGET_UPDATE_INTERVAL_SECS as i64 {
        return None;
    }
    let bytes = storage.get_raw(WINGET_SEARCH_INDEX_KEY).await?;
    let index: Vec<WinGetSearchEntry> = serde_json::from_slice(&bytes).ok()?;
    Some(Arc::new(index))
}
