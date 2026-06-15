use super::{CacheMeta, Storage};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct FsStorage {
    base: PathBuf,
}

impl FsStorage {
    pub fn new(base: &str) -> Self {
        Self {
            base: PathBuf::from(base),
        }
    }

    /// Join `key` under `base`, dropping `..`/`.`/empty components so a
    /// request-derived key cannot escape the cache directory (path traversal).
    fn safe_join(&self, key: &str) -> PathBuf {
        let mut path = self.base.clone();
        for part in key.split('/') {
            if !part.is_empty() && part != "." && part != ".." {
                path.push(part);
            }
        }
        path
    }

    fn data_path(&self, key: &str) -> PathBuf {
        self.safe_join(key)
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        // Meta lives at the "$"-suffixed shadow key (mirrors unstorage's `key + "$"`).
        self.safe_join(&format!("{key}$"))
    }

    async fn ensure_dir(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent).await;
        }
    }
}

#[async_trait]
impl Storage for FsStorage {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        fs::read(self.data_path(key)).await.ok()
    }

    async fn set_raw(&self, key: &str, data: &[u8]) {
        let path = self.data_path(key);
        self.ensure_dir(&path).await;
        let _ = fs::write(path, data).await;
    }

    async fn get_meta(&self, key: &str) -> Option<CacheMeta> {
        let data = fs::read(self.meta_path(key)).await.ok()?;
        serde_json::from_slice(&data).ok()
    }

    async fn set_meta(&self, key: &str, meta: &CacheMeta) {
        let path = self.meta_path(key);
        self.ensure_dir(&path).await;
        if let Ok(data) = serde_json::to_vec(meta) {
            let _ = fs::write(path, data).await;
        }
    }
}
