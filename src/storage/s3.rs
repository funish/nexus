use super::{CacheMeta, Storage};
use async_trait::async_trait;
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::region::Region;
use tracing::error;

/// S3-compatible object storage backend (RustFS, MinIO, etc.) via the lightweight
/// `rust-s3` crate — reuses the shared reqwest/rustls/hyper-1.x HTTP stack instead
/// of aws-sdk-s3's duplicate runtime.
pub struct S3Storage {
    bucket: Box<Bucket>,
}

impl S3Storage {
    pub fn new(config: &crate::config::Config) -> Self {
        let credentials = Credentials::new(
            config.s3_access_key_id.as_deref(),
            config.s3_secret_access_key.as_deref(),
            None,
            None,
            None,
        )
        .expect("failed to build S3 credentials");

        // Region::Custom points rust-s3 at a self-hosted endpoint (RustFS) instead of
        // AWS. Path-style addressing (`/<bucket>/<key>`) is RustFS's default.
        let region = Region::Custom {
            region: config
                .s3_region
                .clone()
                .unwrap_or_else(|| "us-east-1".to_string()),
            endpoint: config.s3_endpoint.clone().expect("S3_ENDPOINT is required"),
        };

        let bucket = Bucket::new(
            config
                .s3_bucket
                .clone()
                .expect("S3_BUCKET is required")
                .as_str(),
            region,
            credentials,
        )
        .expect("failed to build S3 bucket")
        .with_path_style();

        Self { bucket }
    }
}

#[async_trait]
impl Storage for S3Storage {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        match self.bucket.get_object(key).await {
            Ok(resp) if resp.status_code() == 200 => Some(resp.to_vec()),
            // 404 (NoSuchKey) and auth/connector errors are all cache misses here —
            // distinguishing them would only matter for surfacing a 5xx, but the
            // trait contract returns Option so callers treat both as "not cached".
            _ => None,
        }
    }

    async fn set_raw(&self, key: &str, data: &[u8]) {
        if let Err(e) = self.bucket.put_object(key, data).await {
            error!("S3 put_raw failed for {key}: {e:?}");
        }
    }

    async fn get_meta(&self, key: &str) -> Option<CacheMeta> {
        // Meta lives at the "$"-suffixed shadow key (mirrors unstorage's `key + "$"`).
        let meta_key = format!("{key}$");
        let data = self.get_raw(&meta_key).await?;
        serde_json::from_slice(&data).ok()
    }

    async fn set_meta(&self, key: &str, meta: &CacheMeta) {
        let meta_key = format!("{key}$");
        if let Ok(data) = serde_json::to_vec(meta) {
            self.set_raw(&meta_key, &data).await;
        }
    }
}
