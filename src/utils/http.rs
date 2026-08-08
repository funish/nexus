//! Shared HTTP client — one connection pool reused across the whole service
//! (CDN registry/tarball/raw fetches and winget manifest/tree/msix fetches).
//! reqwest pools connections per host, so a single Client lets the CDN and
//! winget — which both hit raw.githubusercontent.com — share the same pool
//! instead of holding two. No global timeout: callers set per-request timeouts
//! via RequestBuilder::timeout, since CDN (15s) and winget (30–120s) need
//! different budgets.

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Result;

pub static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("Funish Nexus")
        .build()
        .expect("failed to build HTTP client")
});

/// Attempts beyond the initial request when the upstream returns a retryable
/// status. Caps total tries at `MAX_RETRIES + 1` so a cold start can't stall.
const MAX_RETRIES: u32 = 2;

/// Transient upstream statuses worth retrying: 429 (rate limit — including the
/// 2025-05 raw.githubusercontent.com limit) and the 5xx family.
fn should_retry(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

/// `Retry-After` as a delta duration (delta-seconds form). The HTTP-date form
/// is vanishingly rare for these statuses, so it is ignored.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Cached `GITHUB_TOKEN` (if set). Authenticated requests get a higher rate
/// limit on both api.github.com (5000/h vs 60/h anonymous) and
/// raw.githubusercontent.com, so every GitHub fetch — winget manifest/tree and
/// CDN tag lookups — passes this through.
pub static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()));

/// GET `url` with bounded retry on 429/5xx. Honors `Retry-After` when present,
/// otherwise exponential backoff (200ms, 400ms). `auth_token`, when given, is
/// sent as `Authorization: Bearer <token>`; `headers` are added to every attempt
/// (e.g. GitHub's `Accept`). Returns the final response — success or the last
/// retryable failure — so the caller owns body/status handling. Every upstream
/// GET goes through here so CDN and winget share one resilient path instead of
/// each call site retrying ad hoc.
pub async fn get_with_retry(
    url: &str,
    timeout: Duration,
    auth_token: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<reqwest::Response> {
    let bearer = auth_token.map(|t| format!("Bearer {t}"));
    let mut attempt = 0u32;
    loop {
        let mut req = HTTP_CLIENT.get(url).timeout(timeout);
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        if let Some(ref auth) = bearer {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() || !should_retry(status) || attempt >= MAX_RETRIES {
            return Ok(resp);
        }
        let wait = retry_after(resp.headers())
            .unwrap_or_else(|| Duration::from_millis(200 << attempt));
        attempt += 1;
        tokio::time::sleep(wait).await;
    }
}
