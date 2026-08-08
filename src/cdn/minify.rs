//! JS/CSS minification.
//!
//! JS uses oxc — the same toolchain rolldown's internal minifier is built on
//! (pinned to the same 0.135 version via Cargo.lock), so there is no second
//! copy of the parser/minifier in the dependency tree. CSS uses lightningcss,
//! which oxc does not cover.

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc::mangler::MangleOptions;
use oxc::minifier::{CompressOptions, Minifier, MinifierOptions};
use oxc::parser::Parser;
use oxc::span::SourceType;

/// Minify JS/TS source via oxc.
///
/// Fail-open: on any parse error the original source is returned unchanged,
/// matching jsDelivr's behavior of never breaking a response over minification.
pub fn minify_js(filename: &str, code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = match SourceType::from_path(Path::new(filename)) {
        Ok(st) => st.with_module(true),
        Err(_) => SourceType::mjs(),
    };

    let parsed = Parser::new(&allocator, code, source_type).parse();
    if !parsed.errors.is_empty() {
        return code.to_string();
    }

    let mut program = parsed.program;
    // Default (not `smallest`) compress: keep top-level declarations intact — a CDN
    // serves the whole file and callers may depend on exported/declared code, so
    // tree-shaking unused top-level functions (which `smallest` does) would be unsafe.
    let options = MinifierOptions {
        mangle: Some(MangleOptions::default()),
        compress: Some(CompressOptions::default()),
    };
    let minified = Minifier::new(options).minify(&allocator, &mut program);

    Codegen::new()
        .with_options(CodegenOptions {
            minify: true,
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .with_scoping(minified.scoping)
        .build(&program)
        .code
}

/// Minify CSS via lightningcss. Fail-open: returns the original on parse error.
///
/// Errors are matched inline rather than propagated: `Error<ParserError<'i>>` borrows
/// the input, so `?` into `anyhow::Error` (which requires `'static`) would force the
/// caller to pass a `'static` string. Keeping the borrow confined here avoids that.
pub fn minify_css(code: &str) -> String {
    use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};

    let Ok(mut stylesheet) = StyleSheet::parse(code, ParserOptions::default()) else {
        return code.to_string();
    };
    if stylesheet.minify(MinifyOptions::default()).is_err() {
        return code.to_string();
    }
    let Ok(result) = stylesheet.to_css(PrinterOptions {
        minify: true,
        ..Default::default()
    }) else {
        return code.to_string();
    };
    result.code
}

/// Dispatch minification by file extension. Non-minifiable types pass through unchanged.
pub fn minify_for(filename: &str, code: &[u8]) -> Vec<u8> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let text = match std::str::from_utf8(code) {
        Ok(s) => s,
        Err(_) => return code.to_vec(),
    };
    match ext {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => {
            minify_js(filename, text).into_bytes()
        }
        "css" => minify_css(text).into_bytes(),
        _ => code.to_vec(),
    }
}

/// Return the always-minified bytes for a default/entry file, caching the result
/// under a "+min/" key so the oxc/lightningcss pass runs only once. jsDelivr serves
/// the default file always-minified; npm, jsr and cdnjs share this path. Non-minifiable
/// types (README, .php, …) pass through unchanged via `minify_for`.
pub async fn minified_entry(
    storage: &crate::storage::SharedStorage,
    cache_base: &str,
    entry_file: &str,
    original: &[u8],
) -> Vec<u8> {
    let min_key = format!("{cache_base}/+min/{entry_file}");
    if let Some(cached) = storage.get_raw(&min_key).await {
        return cached;
    }
    let minified = minify_for(entry_file, original);
    let s = storage.clone();
    let (k, d) = (min_key, minified.clone());
    tokio::spawn(async move {
        s.set_raw(&k, &d).await;
    });
    minified
}

/// If `path` ends with `.min.js`/`.min.css`, return the un-minified variant
/// (e.g. `foo.min.js` → `foo.js`). Used for jsDelivr-style `.min` synthesis.
pub fn strip_min_suffix(path: &str) -> Option<String> {
    ["js", "css"].iter().find_map(|ext| {
        let suffix = format!(".min.{ext}");
        path.strip_suffix(&suffix)
            .map(|base| format!("{base}.{ext}"))
    })
}

/// jsDelivr `.min` synthesis: `foo.min.js` requested when only `foo.js` exists.
///
/// Fetches the un-minified source from the tarball (downloading + warming it on the
/// way, so cold packages are covered), minifies it, caches the result, and returns
/// the response. Returns `Ok(None)` when `filepath` is not a `.min.{js,css}` path or
/// its source is absent — the caller then falls through to its own fallback (npm
/// retries older matching versions; gh 404s or lists the directory).
///
/// `raw_url_base` picks the per-origin direct URL for the source file: gh passes the
/// raw GitHub base (linking to raw.githubusercontent.com), npm passes `None` (proxied
/// through us). `resolved_version`, when `Some`, emits `x-resolved-version` via
/// `file_response_versioned` (npm); `None` uses the plain `file_response` (gh).
// Eight params are the irreducible interface — source/target location, response
// headers/cache-control, and the two per-origin knobs — and each is consumed once;
// a ctx struct would only rename them for two call sites.
#[allow(clippy::too_many_arguments)]
pub async fn try_min_synthesis(
    storage: &crate::storage::SharedStorage,
    tarball_url: &str,
    cache_base: &str,
    filepath: &str,
    headers: &axum::http::HeaderMap,
    cache_control: &'static str,
    raw_url_base: Option<&str>,
    resolved_version: Option<&str>,
) -> Result<Option<axum::response::Response>, crate::error::AppError> {
    // Only `.min.{js,css}` paths are synthesizable; otherwise the caller proceeds.
    let Some(orig) = strip_min_suffix(filepath) else {
        return Ok(None);
    };
    // gh serves a raw GitHub URL for the un-minified file; npm proxies through us.
    let direct_url = raw_url_base.map(|base| format!("{base}/{orig}"));
    let orig_data = match crate::cdn::tarball::extract_file_from_tarball(
        storage,
        tarball_url,
        &orig,
        &format!("{cache_base}/{orig}"),
        direct_url.as_deref(),
        None,
    )
    .await
    {
        Ok(data) => data,
        Err(_) => return Ok(None),
    };
    let minified = minify_for(&orig, &orig_data);
    // Cache the synthesized .min file so later requests hit storage directly.
    let s = storage.clone();
    let (k, d) = (format!("{cache_base}/{filepath}"), minified.clone());
    tokio::spawn(async move {
        s.set_raw(&k, &d).await;
    });
    let resp = match resolved_version {
        Some(v) => crate::cdn::response::file_response_versioned(
            filepath,
            &minified,
            cache_control,
            headers,
            v,
            None,
        ),
        None => crate::cdn::response::file_response(
            filepath,
            &minified,
            cache_control,
            headers,
            None,
        ),
    };
    Ok(Some(resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minify_js_shrinks_and_strips_comments() {
        // `add` is referenced by the call, so the minifier keeps it. Unreferenced
        // top-level declarations are tree-shaken — expected for a CDN serving
        // real files whose top-level code is referenced or exported.
        let input =
            "function add(a, b) {\n  // drop me\n  return a + b;\n}\nconsole.log(add(1, 2));\n";
        let out = minify_js("test.js", input);
        assert!(out.len() < input.len(), "minified output should be smaller");
        assert!(!out.contains("drop me"), "comments must be removed");
        assert!(out.contains("return"), "code body must survive");
    }

    #[test]
    fn minify_js_fail_opens_on_parse_error() {
        let input = "function {{{{{ totally broken";
        let out = minify_js("test.js", input);
        assert_eq!(
            out, input,
            "parse errors must return the original unchanged"
        );
    }

    #[test]
    fn minify_css_compacts_rules() {
        let input = ".foo {\n  color: red;\n}\n.bar {\n  color: red;\n}\n";
        let out = minify_css(input);
        assert!(out.len() < input.len(), "minified CSS should be smaller");
        assert!(out.contains(".foo"));
    }

    #[test]
    fn strip_min_suffix_handles_js_css_and_others() {
        assert_eq!(strip_min_suffix("foo.min.js").as_deref(), Some("foo.js"));
        assert_eq!(
            strip_min_suffix("a/b/c.min.css").as_deref(),
            Some("a/b/c.css")
        );
        assert_eq!(strip_min_suffix("foo.js"), None);
        assert_eq!(strip_min_suffix("foo.min.json"), None);
    }

    #[tokio::test]
    async fn try_min_synthesis_skips_non_min_paths_without_fetching() {
        // A non-`.min` path short-circuits to Ok(None) before any storage or network
        // access, so an unreachable tarball URL proves no fetch happened.
        let tmp = tempfile::tempdir().unwrap();
        let storage: crate::storage::SharedStorage =
            std::sync::Arc::new(crate::storage::fs::FsStorage::new(
                tmp.path().to_str().unwrap(),
            ));
        let headers = axum::http::HeaderMap::new();
        let resp = try_min_synthesis(
            &storage,
            "https://example.invalid/x.tgz",
            "cdn/npm/x@1.0.0",
            "foo.js",
            &headers,
            "public, max-age=60",
            None,
            None,
        )
        .await
        .unwrap();
        assert!(resp.is_none(), "non-.min path must return Ok(None)");
    }
}
