use super::{CacheMeta, Storage};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

    async fn ensure_dir(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            return fs::create_dir_all(parent).await;
        }
        Ok(())
    }

    /// Unique sibling temp path for an atomic write-then-rename. Appends to the
    /// file name (extension-preserving) so distinct keys never collide.
    fn tmp_path(path: &Path) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut name = path
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        name.push(format!(".tmp{}-{}", std::process::id(), n));
        path.with_file_name(name)
    }
}

#[async_trait]
impl Storage for FsStorage {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        fs::read(self.data_path(key)).await.ok()
    }

    async fn set_raw(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        let path = self.data_path(key);
        self.ensure_dir(&path).await?;
        // Write to a unique temp file then rename so concurrent readers never
        // observe a truncated or half-written value. tokio's rename maps to
        // MoveFileEx(REPLACE_EXISTING) on Windows and rename(2) on Unix, both of
        // which atomically replace an existing target.
        let tmp = Self::tmp_path(&path);
        if let Err(e) = fs::write(&tmp, data).await {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::anyhow!("fs write failed for {key}: {e}"));
        }
        if let Err(e) = fs::rename(&tmp, &path).await {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::anyhow!("fs write failed for {key}: {e}"));
        }
        Ok(())
    }

    async fn get_meta(&self, key: &str) -> Option<CacheMeta> {
        let data = fs::read(self.meta_path(key)).await.ok()?;
        serde_json::from_slice(&data).ok()
    }

    async fn set_meta(&self, key: &str, meta: &CacheMeta) -> anyhow::Result<()> {
        let path = self.meta_path(key);
        self.ensure_dir(&path).await?;
        let data = serde_json::to_vec(meta)?;
        let tmp = Self::tmp_path(&path);
        if let Err(e) = fs::write(&tmp, data).await {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::anyhow!("fs meta write failed for {key}: {e}"));
        }
        if let Err(e) = fs::rename(&tmp, &path).await {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::anyhow!("fs meta write failed for {key}: {e}"));
        }
        Ok(())
    }
}
