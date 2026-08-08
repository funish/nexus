//! Content-type detection by file extension, backed by the `mime_guess` crate.
//!
//! The JavaScript/TypeScript family is forced to a JavaScript MIME because
//! `mime_guess` classifies `.ts`/`.tsx` as `video/mp2t`. Text-oriented web types
//! are served with a `charset=utf-8` suffix to match browser expectations.

use mime_guess::mime;

/// Guess the content type for a path, appending `charset=utf-8` to text types.
pub fn get_content_type(filepath: &str) -> String {
    let ext = filepath.rsplit('.').next().unwrap_or("");

    // The JS/TS family is always JavaScript; mime_guess misclassifies .ts/.tsx/etc.
    if matches!(ext, "js" | "mjs" | "cjs" | "ts" | "mts" | "jsx" | "tsx") {
        return "application/javascript; charset=utf-8".to_string();
    }

    // jsDelivr serves HTML as text/plain for security (prevent XSS).
    if matches!(ext, "html" | "htm") {
        return "text/plain; charset=utf-8".to_string();
    }

    match mime_guess::from_path(filepath).first() {
        Some(m) if needs_charset(&m) => format!("{m}; charset=utf-8"),
        Some(m) => m.to_string(),
        None => "application/octet-stream".to_string(),
    }
}

/// Whether a MIME type should carry a UTF-8 charset (text/* plus common web types).
fn needs_charset(m: &mime_guess::Mime) -> bool {
    m.type_() == mime::TEXT || matches!(m.subtype().as_ref(), "javascript" | "json" | "xml")
}
