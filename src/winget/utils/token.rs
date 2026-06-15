//! ContinuationToken encode/decode — a base64-encoded numeric offset (mirrors token.ts).

use base64::{Engine, engine::general_purpose::STANDARD};

/// Encode an offset into a continuation token.
pub fn encode_continuation_token(offset: usize) -> String {
    STANDARD.encode(offset.to_string())
}

/// Decode a continuation token into an offset; missing or malformed input returns 0.
pub fn decode_continuation_token(token: Option<&str>) -> usize {
    let Some(t) = token else { return 0 };
    STANDARD
        .decode(t.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(
            decode_continuation_token(Some(&encode_continuation_token(100))),
            100
        );
        assert_eq!(decode_continuation_token(None), 0);
        assert_eq!(decode_continuation_token(Some("!!!invalid")), 0);
    }
}
