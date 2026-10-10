pub mod fs;
pub mod s3;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheMeta {
    pub files: Option<Vec<CdnFileMeta>>,
    pub skipped_at: Option<u64>,
    pub mtime: Option<String>,
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdnFileMeta {
    pub name: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
}

#[async_trait]
pub trait Storage: Send + Sync {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>>;
    /// Writes must report failure: a silently-dropped write followed by a fresh
    /// mtime or file-list meta would pin an empty entry until its TTL expires.
    async fn set_raw(&self, key: &str, data: &[u8]) -> anyhow::Result<()>;
    async fn get_meta(&self, key: &str) -> Option<CacheMeta>;
    async fn set_meta(&self, key: &str, meta: &CacheMeta) -> anyhow::Result<()>;
}

pub type SharedStorage = Arc<dyn Storage>;

pub async fn create_storage(config: &crate::config::Config) -> SharedStorage {
    if config.has_s3_config() {
        Arc::new(s3::S3Storage::new(config))
    } else {
        Arc::new(fs::FsStorage::new(&config.cache_dir))
    }
}
