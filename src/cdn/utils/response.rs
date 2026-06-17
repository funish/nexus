//! Shared file-response builder with SRI ETag / If-None-Match 304 support.
//!
//! The npm and jsr routes build their responses inline; gh, cdnjs and wp use this
//! helper so they get the same client-cache negotiation (ETag + 304) jsDelivr offers
//! on every response.

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
) -> Response {
    let etag = calculate_integrity(data);
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
