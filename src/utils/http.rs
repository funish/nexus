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
        // Fail fast on TCP connect instead of hanging for the OS default (~2 min).
        .connect_timeout(Duration::from_secs(10))
        // HTTP/2 keep-alive: sends PING frames so dead upstream connections
        // (NAT timeout, proxy drop) are detected instead of hanging until the
        // per-request timeout. Enables connection reuse across CDN + winget.
        .http2_keep_alive_interval(Duration::from_secs(30))
        .http2_keep_alive_timeout(Duration::from_secs(10))
        .http2_keep_alive_while_idle(true)
        // Cap idle connections per host to bound memory. 32 covers all
        // concurrent upstreams (npm/jsr/gh/cdnjs/raw/winget) under the
        // DOWNLOAD_SEMAPHORE limit of 50.
        .pool_max_idle_per_host(32)
        .build()
        .expect("failed to build HTTP client")
});

/// Attempts beyond the initial request when the upstream returns a retryable
/// status. Caps total tries at `MAX_RETRIES + 1` so a cold start can't stall.
const MAX_RETRIES: u32 = 2;

/// Transient upstream statuses worth retrying: 429 (rate limit — including the
/// 2025-05 raw.githubusercontent.com limit), GitHub's secondary signal of 403
/// with an exhausted rate-limit budget, and the 5xx family.
fn should_retry(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap) -> bool {
    if status == reqwest::StatusCode::FORBIDDEN
        && headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0")
    {
        return true;
    }
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

/// Classified download failure so callers can negative-cache deterministic
/// misses (404, oversized) without freezing out transient upstream faults.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("resource not found")]
    NotFound,
    #[error("resource exceeds size limit")]
    TooLarge,
    #[error("download failed after retries: {0}")]
    Transient(#[source] anyhow::Error),
}

/// Body-read failure: oversize is terminal, transport errors may be retried
/// from scratch (the partial bytes are dropped with the response).
enum BodyError {
    TooLarge,
    Transient(anyhow::Error),
}

async fn read_capped_body(
    mut resp: reqwest::Response,
    max_size: u64,
) -> std::result::Result<Vec<u8>, BodyError> {
    let mut body = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > max_size as usize {
                    return Err(BodyError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body),
            Err(e) => return Err(BodyError::Transient(anyhow::anyhow!("{e}"))),
        }
    }
}

/// GET `url` and fully materialize a successful body, retrying from scratch on
/// both retryable statuses and mid-body transport failures (a partial body is
/// discarded with its response). Oversized resources are terminal, not retried.
/// The retry budget is shared with status retries so the worst case stays
/// bounded at MAX_RETRIES + 1 requests.
pub async fn download_to_vec(
    url: &str,
    timeout: Duration,
    max_size: u64,
) -> std::result::Result<Vec<u8>, DownloadError> {
    let mut attempt = 0u32;
    loop {
        let resp = HTTP_CLIENT
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| DownloadError::Transient(anyhow::anyhow!("{e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(DownloadError::NotFound);
        }
        if !status.is_success() {
            if should_retry(status, resp.headers()) && attempt < MAX_RETRIES {
                let wait = retry_after(resp.headers())
                    .unwrap_or_else(|| Duration::from_millis(200 << attempt))
                    .min(Duration::from_secs(5));
                drop(resp);
                attempt += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            return Err(DownloadError::Transient(anyhow::anyhow!(
                "upstream returned {status}"
            )));
        }
        if let Some(len) = resp.content_length()
            && len > max_size
        {
            return Err(DownloadError::TooLarge);
        }
        return match read_capped_body(resp, max_size).await {
            Ok(bytes) => Ok(bytes),
            Err(BodyError::TooLarge) => Err(DownloadError::TooLarge),
            Err(BodyError::Transient(e)) => {
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
                    continue;
                }
                Err(DownloadError::Transient(e))
            }
        };
    }
}

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
        if status.is_success() || !should_retry(status, resp.headers()) || attempt >= MAX_RETRIES {
            return Ok(resp);
        }
        // Cap the wait: the caller already holds a download permit for this
        // request, so a hostile `Retry-After: 3600` must not pin the slot.
        let wait = retry_after(resp.headers())
            .unwrap_or_else(|| Duration::from_millis(200 << attempt))
            .min(Duration::from_secs(5));
        attempt += 1;
        tokio::time::sleep(wait).await;
    }
}
