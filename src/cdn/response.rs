//! Shared file-response builders with SRI ETag / If-None-Match 304 support.
//!
//! All CDN file routes (npm, jsr, gh, cdnjs, wp) build their responses through
//! these helpers, so every response carries the ETag + 304 client-cache
//! negotiation jsDelivr offers.

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::integrity::calculate_integrity;
use super::mime::get_content_type;

/// jsDelivr-style weak ETag `W/"<len>-<hash>"`. The hash half is the SRI digest
/// already computed (and cached in package meta), so no extra hashing happens —
/// only the presentation differs from the `sha256-` form.
fn weak_etag(len: usize, integrity: &str) -> String {
    let hash = integrity.strip_prefix("sha256-").unwrap_or(integrity);
    format!("W/\"{len}-{hash}\"")
}

/// RFC 7232 weak comparison for `If-None-Match`: `*` matches any existing
/// resource, comma-separated tag lists are honored across repeated header
/// instances, and `W/"x"` compares equal to `"x"`.
fn if_none_match_matches(headers: &HeaderMap, etag: &str) -> bool {
    let mut any = false;
    for value in headers.get_all("if-none-match") {
        let Ok(v) = value.to_str() else { continue };
        for candidate in v.split(',') {
            let candidate = candidate.trim();
            if candidate == "*" {
                return true;
            }
            // Weak comparison: strip the validator's own W/ prefix and the
            // candidate's, then compare the opaque tags byte for byte.
            let candidate = candidate.strip_prefix("W/").unwrap_or(candidate);
            if candidate == etag.strip_prefix("W/").unwrap_or(etag) {
                return true;
            }
            any = true;
        }
    }
    // An If-None-Match header present but unparseable must not 304.
    let _ = any;
    false
}

/// Return a 304 when the client's `If-None-Match` matches `etag` under the
/// RFC 7232 weak comparison, otherwise `None`.
pub fn if_none_match_304(
    headers: &HeaderMap,
    etag: &str,
    cache_control: &'static str,
) -> Option<Response> {
    if if_none_match_matches(headers, etag) {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(v) = HeaderValue::from_str(etag) {
            resp.headers_mut().insert("etag", v);
        }
        // A 304 must carry the same cache validators its 200 would, so the
        // client keeps the same caching policy it already applied.
        resp.headers_mut()
            .insert("cache-control", HeaderValue::from_static(cache_control));
        resp.headers_mut()
            .insert("vary", HeaderValue::from_static("Accept-Encoding"));
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
    let etag = weak_etag(
        data.len(),
        &etag
            .map(str::to_string)
            .unwrap_or_else(|| calculate_integrity(data)),
    );
    if let Some(resp) = if_none_match_304(headers, &etag, cache_control) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn inm(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("if-none-match", HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn exact_tag_matches() {
        assert!(if_none_match_matches(&inm("\"5-abc\""), "W/\"5-abc\""));
    }

    #[test]
    fn weak_prefix_is_ignored_on_both_sides() {
        assert!(if_none_match_matches(&inm("W/\"5-abc\""), "W/\"5-abc\""));
        assert!(if_none_match_matches(&inm("\"5-abc\""), "\"5-abc\""));
    }

    #[test]
    fn tag_list_and_star_match() {
        assert!(if_none_match_matches(
            &inm("\"1-a\", \"5-abc\""),
            "W/\"5-abc\""
        ));
        assert!(if_none_match_matches(&inm("*"), "W/\"5-abc\""));
    }

    #[test]
    fn mismatched_and_garbage_do_not_match() {
        assert!(!if_none_match_matches(&inm("\"1-a\""), "W/\"5-abc\""));
        assert!(!if_none_match_matches(&inm("garbage"), "W/\"5-abc\""));
    }
}
