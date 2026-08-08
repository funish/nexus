//! Shared file-response builders with SRI ETag / If-None-Match 304 support.
//!
//! All CDN file routes (npm, jsr, gh, cdnjs, wp) build their responses through
//! these helpers, so every response carries the ETag + 304 client-cache
//! negotiation jsDelivr offers.

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::integrity::calculate_integrity;
use super::mime::get_content_type;

/// Return a 304 when the client's `If-None-Match` equals `etag`, otherwise `None`.
pub fn if_none_match_304(headers: &HeaderMap, etag: &str) -> Option<Response> {
    let matches = headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag);
    if matches {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(v) = HeaderValue::from_str(etag) {
            resp.headers_mut().insert("etag", v);
        }
        return Some(resp);
    }
    None
}

/// Build a 200 file response: an SRI ETag, an `If-None-Match` 304 short-circuit,
/// content-type, and cache-control.
pub fn file_response(
    filename: &str,
    data: &[u8],
    cache_control: &'static str,
    headers: &HeaderMap,
    etag: Option<&str>,
) -> Response {
    // Reuse a precomputed integrity (e.g. from the cached package meta) when
    // available; otherwise hash the body. Avoids re-running SHA-256 on every
    // request for a file whose integrity is already cached.
    let etag = etag
        .map(str::to_string)
        .unwrap_or_else(|| calculate_integrity(data));
    if let Some(resp) = if_none_match_304(headers, &etag) {
        return resp;
    }
    let mut resp = (
        StatusCode::OK,
        [
            ("cache-control", cache_control),
            ("vary", "Accept-Encoding"),
            ("etag", etag.as_str()),
        ],
        data.to_vec(),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&get_content_type(filename)) {
        resp.headers_mut().insert("content-type", v);
    }
    resp
}

/// Like [`file_response`], but also stamps an `x-resolved-version` header so a
/// client can see which concrete version an alias/range request resolved to.
/// Used by the npm/jsr routes.
pub fn file_response_versioned(
    filename: &str,
    data: &[u8],
    cache_control: &'static str,
    headers: &HeaderMap,
    resolved_version: &str,
    etag: Option<&str>,
) -> Response {
    let mut resp = file_response(filename, data, cache_control, headers, etag);
    if let Ok(v) = HeaderValue::from_str(resolved_version) {
        resp.headers_mut().insert("x-resolved-version", v);
    }
    resp
}
