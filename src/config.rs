use std::env;
use std::sync::OnceLock;

use url::Url;

pub struct Config {
    pub port: u16,
    pub cache_dir: String,
    pub winget_source_identifier: String,
    pub winget_source_msix_url: String,
    pub winget_github_repo: String,
    pub winget_github_branch: String,
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
            winget_source_identifier: configured_value("WINGET_SOURCE_IDENTIFIER", "Funish.Nexus"),
            winget_source_msix_url: configured_value(
                "WINGET_SOURCE_MSIX_URL",
                "https://cdn.winget.microsoft.com/cache/source.msix",
            ),
            winget_github_repo: configured_value("WINGET_GITHUB_REPO", "microsoft/winget-pkgs"),
            winget_github_branch: configured_value("WINGET_GITHUB_BRANCH", "master"),
            s3_access_key_id: env::var("S3_ACCESS_KEY_ID").ok(),
            s3_secret_access_key: env::var("S3_SECRET_ACCESS_KEY").ok(),
            s3_endpoint: env::var("S3_ENDPOINT").ok(),
            s3_region: env::var("S3_REGION").ok(),
            s3_bucket: env::var("S3_BUCKET").ok(),
        }
    }

    pub fn validate(&self) {
        let identifier_length = self.winget_source_identifier.chars().count();
        assert!(
            (3..=128).contains(&identifier_length),
            "WINGET_SOURCE_IDENTIFIER must contain 3 to 128 characters"
        );

        let source_url = Url::parse(&self.winget_source_msix_url)
            .unwrap_or_else(|_| panic!("WINGET_SOURCE_MSIX_URL must be a valid URL"));
        assert!(
            matches!(source_url.scheme(), "http" | "https"),
            "WINGET_SOURCE_MSIX_URL must use HTTP or HTTPS"
        );

        let repo_parts = self.winget_github_repo.split('/').count();
        assert!(
            repo_parts == 2
                && self
                    .winget_github_repo
                    .split('/')
                    .all(|part| !part.trim().is_empty()),
            "WINGET_GITHUB_REPO must be an owner/repository pair"
        );

        assert!(
            !self.winget_github_branch.trim().is_empty(),
            "WINGET_GITHUB_BRANCH must not be empty"
        );
    }

    pub fn has_s3_config(&self) -> bool {
        self.s3_access_key_id.is_some()
            && self.s3_secret_access_key.is_some()
            && self.s3_endpoint.is_some()
            && self.s3_bucket.is_some()
    }
}

fn configured_value(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| default.to_string())
}

#[derive(Clone)]
struct WingetRuntime {
    source_identifier: String,
    source_msix_url: String,
    github_repo: String,
    github_branch: String,
}

static WINGET_RUNTIME: OnceLock<WingetRuntime> = OnceLock::new();

/// Capture process-wide WinGet source settings after configuration is validated.
pub fn set_winget_runtime(config: &Config) {
    config.validate();
    let _ = WINGET_RUNTIME.set(WingetRuntime {
        source_identifier: config.winget_source_identifier.clone(),
        source_msix_url: config.winget_source_msix_url.clone(),
        github_repo: config.winget_github_repo.clone(),
        github_branch: config.winget_github_branch.clone(),
    });
}

pub fn winget_source_identifier() -> &'static str {
    runtime().source_identifier.as_str()
}

pub fn winget_source_msix_url() -> &'static str {
    runtime().source_msix_url.as_str()
}

pub fn winget_github_repo() -> &'static str {
    runtime().github_repo.as_str()
}

pub fn winget_github_branch() -> &'static str {
    runtime().github_branch.as_str()
}

fn runtime() -> &'static WingetRuntime {
    WINGET_RUNTIME.get().unwrap_or_else(|| {
        let config = Config::from_env();
        config.validate();
        let _ = WINGET_RUNTIME.set(WingetRuntime {
            source_identifier: config.winget_source_identifier,
            source_msix_url: config.winget_source_msix_url,
            github_repo: config.winget_github_repo,
            github_branch: config.winget_github_branch,
        });
        WINGET_RUNTIME.get().unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_requires_a_github_owner_and_repository() {
        let config = Config {
            port: 0,
            cache_dir: String::new(),
            winget_source_identifier: "Test.Source".to_string(),
            winget_source_msix_url: "https://example.invalid/source.msix".to_string(),
            winget_github_repo: "owner".to_string(),
            winget_github_branch: "main".to_string(),
            s3_access_key_id: None,
            s3_secret_access_key: None,
            s3_endpoint: None,
            s3_region: None,
            s3_bucket: None,
        };
        assert!(std::panic::catch_unwind(|| config.validate()).is_err());
    }
}
