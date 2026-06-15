use super::{CacheMeta, Storage};
use async_trait::async_trait;
use aws_sdk_s3::Client;
use tracing::error;

pub struct S3Storage {
    client: Client,
    bucket: String,
}

impl S3Storage {
    pub fn new(config: &crate::config::Config) -> Self {
        let aws_config = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
                if let Some(ref endpoint) = config.s3_endpoint {
                    loader = loader.endpoint_url(endpoint);
                }
                if let Some(ref region) = config.s3_region {
                    loader = loader.region(aws_config::Region::new(region.clone()));
                }
                loader.load().await
            })
        });

        let client = Client::new(&aws_config);
        Self {
            client,
            bucket: config.s3_bucket.clone().unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Storage for S3Storage {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => {
                let bytes = output.body.collect().await.ok()?.into_bytes();
                Some(bytes.to_vec())
            }
            Err(_) => None,
        }
    }

    async fn set_raw(&self, key: &str, data: &[u8]) {
        match self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(data.to_vec().into())
            .send()
            .await
        {
            Ok(_) => {}
            Err(e) => error!("S3 put_raw failed for {key}: {e}"),
        }
    }

    async fn get_meta(&self, key: &str) -> Option<CacheMeta> {
        // Meta is a "$"-suffixed shadow key (mirrors unstorage's `key + "$"`).
        let meta_key = format!("{key}$");
        let data = self.get_raw(&meta_key).await?;
        serde_json::from_slice(&data).ok()
    }

    async fn set_meta(&self, key: &str, meta: &CacheMeta) {
        // Meta is a "$"-suffixed shadow key (mirrors unstorage's `key + "$"`).
        let meta_key = format!("{key}$");
        if let Ok(data) = serde_json::to_vec(meta) {
            self.set_raw(&meta_key, &data).await;
        }
    }
}
