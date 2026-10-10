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
        // Cache idle sockets for the configured cold-start width. This bounds
        // idle memory only; reqwest never caps active concurrent requests here.
        .pool_max_idle_per_host(crate::utils::concurrency::download_concurrency())
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

/// Cached `GITHUB_TOKEN` (if set). Only `api.github.com` consumes it: the token
/// raises the primary quota from 60 to 5,000 requests/hour. Raw content and
/// codeload archives are separate endpoints and are deliberately left
/// unauthenticated.
pub static GITHUB_TOKEN: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()));

/// A successful or terminal response whose body has already been materialized.
/// Callers can inspect status/headers (for example `304` + `ETag`) without
/// re-reading a streamed body after a transient transport failure.
pub struct FetchedResponse {
    pub status: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

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

/// Internal classification needed because oversize is a deterministic caller
/// error while transport failures are retryable/public-service faults.
enum FetchError {
    TooLarge,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for FetchError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value)
    }
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
    let fetched = fetch_with_retry(url, timeout, None, &[], max_size)
        .await
        .map_err(|e| match e {
            FetchError::TooLarge => DownloadError::TooLarge,
            FetchError::Other(e) => DownloadError::Transient(e),
        })?;
    if fetched.status == reqwest::StatusCode::NOT_FOUND {
        return Err(DownloadError::NotFound);
    }
    if !fetched.status.is_success() {
        return Err(DownloadError::Transient(anyhow::anyhow!(
            "upstream returned {}",
            fetched.status
        )));
    }
    Ok(fetched.body)
}

async fn send_get(
    url: &str,
    timeout: Duration,
    auth_token: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<reqwest::Response> {
    let bearer = auth_token.map(|t| format!("Bearer {t}"));
    let mut req = HTTP_CLIENT.get(url).timeout(timeout);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    if let Some(ref auth) = bearer {
        req = req.header("Authorization", auth);
    }
    req.send().await.map_err(Into::into)
}

fn retry_wait(headers: &reqwest::header::HeaderMap, attempt: u32) -> Duration {
    retry_after(headers)
        .unwrap_or_else(|| Duration::from_millis(200 << attempt))
        .min(Duration::from_secs(5))
}

async fn fetch_with_retry(
    url: &str,
    timeout: Duration,
    auth_token: Option<&str>,
    headers: &[(&str, &str)],
    max_size: u64,
) -> std::result::Result<FetchedResponse, FetchError> {
    let mut attempt = 0u32;
    loop {
        let resp = send_get(url, timeout, auth_token, headers)
            .await
            .map_err(FetchError::Other)?;
        let status = resp.status();
        let response_headers = resp.headers().clone();
        if !status.is_success() {
            if should_retry(status, resp.headers()) && attempt < MAX_RETRIES {
                let wait = retry_wait(resp.headers(), attempt);
                drop(resp);
                attempt += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            return Ok(FetchedResponse {
                status,
                headers: response_headers,
                body: Vec::new(),
            });
        }
        if let Some(len) = resp.content_length()
            && len > max_size
        {
            return Err(FetchError::TooLarge);
        }
        return match read_capped_body(resp, max_size).await {
            Ok(body) => Ok(FetchedResponse {
                status,
                headers: response_headers,
                body,
            }),
            Err(BodyError::TooLarge) => Err(FetchError::TooLarge),
            Err(BodyError::Transient(e)) => {
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
                    continue;
                }
                Err(FetchError::Other(e))
            }
        };
    }
}

/// GET `url` and materialize its body before returning, with bounded retry on
/// both 429/5xx statuses and mid-body transport failures. Non-success responses
/// keep their status/headers but have no body, which also covers conditional
/// `304` handling. `auth_token`, when given, is sent as `Authorization: Bearer
/// <token>`; `headers` are added to every attempt. Every fully-consumed upstream
/// GET goes through here so CDN and winget share one resilient path instead of
/// each call site retrying ad hoc.
pub async fn get_fetched_with_retry(
    url: &str,
    timeout: Duration,
    auth_token: Option<&str>,
    headers: &[(&str, &str)],
    max_size: u64,
) -> Result<FetchedResponse> {
    fetch_with_retry(url, timeout, auth_token, headers, max_size)
        .await
        .map_err(|e| match e {
            FetchError::TooLarge => anyhow::anyhow!("resource exceeds size limit"),
            FetchError::Other(e) => e,
        })
}
