//! Shared HTTP client — a single connection pool reused across all winget
//! fetches (manifest files, GitHub trees, source.msix download). Per-request
//! timeouts are set on the RequestBuilder instead of the client so each caller
//! keeps its own budget.

use std::sync::LazyLock;

pub static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("Funish Nexus")
        .build()
        .expect("failed to build winget HTTP client")
});
