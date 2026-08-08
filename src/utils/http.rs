//! Shared HTTP client — one connection pool reused across the whole service
//! (CDN registry/tarball/raw fetches and winget manifest/tree/msix fetches).
//! reqwest pools connections per host, so a single Client lets the CDN and
//! winget — which both hit raw.githubusercontent.com — share the same pool
//! instead of holding two. No global timeout: callers set per-request timeouts
//! via RequestBuilder::timeout, since CDN (15s) and winget (30–120s) need
//! different budgets.

use std::sync::LazyLock;

pub static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("Funish Nexus")
        .build()
        .expect("failed to build HTTP client")
});
