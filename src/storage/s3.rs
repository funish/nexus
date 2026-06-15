use super::{CacheMeta, Storage};
use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use tracing::error;

pub struct S3Storage {
    client: Client,
    bucket: String,
}

impl S3Storage {
    pub async fn new(config: &crate::config::Config) -> Self {
        // Static credentials from config. The S3_* env vars don't match the
        // AWS_* names aws-config's default chain reads, so without this the
        // chain falls back to IMDS (169.254.169.254), which is unreachable in
        // most containers and surfaces as a "dispatch failure" on every request.
        let credentials = Credentials::new(
            config.s3_access_key_id.clone().unwrap_or_default(),
            config.s3_secret_access_key.clone().unwrap_or_default(),
            None,
            None,
            "static",
        );

        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(credentials);
        if let Some(ref endpoint) = config.s3_endpoint {
            loader = loader.endpoint_url(endpoint);
        }
        if let Some(ref region) = config.s3_region {
            loader = loader.region(aws_config::Region::new(region.clone()));
        }
        let aws_config = loader.load().await;

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
            // {:?} surfaces the full error chain (connector / TLS / DNS / signing);
            // the short Display form just prints "dispatch failure".
            Err(e) => error!("S3 put_raw failed for {key}: {e:?}"),
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
