use std::env;

pub struct Config {
    pub port: u16,
    pub cache_dir: String,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub s3_bucket: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            cache_dir: env::var("CACHE_DIR").unwrap_or_else(|_| "./.cache".to_string()),
            s3_access_key_id: env::var("S3_ACCESS_KEY_ID").ok(),
            s3_secret_access_key: env::var("S3_SECRET_ACCESS_KEY").ok(),
            s3_endpoint: env::var("S3_ENDPOINT").ok(),
            s3_region: env::var("S3_REGION").ok(),
            s3_bucket: env::var("S3_BUCKET").ok(),
        }
    }

    pub fn has_s3_config(&self) -> bool {
        self.s3_access_key_id.is_some()
            && self.s3_secret_access_key.is_some()
            && self.s3_endpoint.is_some()
            && self.s3_bucket.is_some()
    }
}
