//! Default file resolution aligned with jsDelivr's package.json priority.

use serde_json::Value;

/// Common entry filenames tried when package.json declares no entry field.
/// Order: most common first (Node CJS, then ESM, then Deno/library conventions).
/// The caller picks the first one actually present in the package.
pub const ENTRY_FALLBACKS: &[&str] = &[
    "index.js",
    "index.mjs",
    "index.cjs",
    "mod.js",
    "mod.ts",
    "lib.js",
];

/// jsDelivr default JS entry priority: `jsdelivr` > `browser` > `main`.
/// Returns `None` when the package declares no entry (caller tries `ENTRY_FALLBACKS`).
pub fn resolve_default_file(pkg: &Value) -> Option<String> {
    for field in &["jsdelivr", "browser", "main"] {
        if let Some(file) = pkg.get(*field).and_then(|v| v.as_str()) {
            return Some(strip_dot_slash(file).to_string());
        }
    }
    None
}

/// Entry resolution for `+esm` bundling. Distinct from `resolve_default_file`
/// because the goal is a browser ESM bundle, so module-aware fields win:
/// `exports["."]` conditions > `browser`(string) > `module` > `jsdelivr` > `main`.
pub fn resolve_esm_entry(pkg: &Value) -> Option<String> {
    // exports["."] — Node's modern entry standard (string form or conditions object).
    if let Some(dot) = pkg.get("exports").and_then(|e| e.get(".")) {
        if let Some(s) = dot.as_str() {
            return Some(strip_dot_slash(s).to_string());
        }
        // Conditions: prefer a browser build, then ESM import, then default.
        for cond in &["browser", "import", "module", "default"] {
            if let Some(s) = dot.get(*cond).and_then(|v| v.as_str()) {
                return Some(strip_dot_slash(s).to_string());
            }
        }
    }
    for field in &["browser", "module", "jsdelivr", "main"] {
        if let Some(s) = pkg.get(*field).and_then(|v| v.as_str()) {
            return Some(strip_dot_slash(s).to_string());
        }
    }
    None
}

/// jsDelivr convention: CSS entry comes from the `style` field.
pub fn resolve_style_file(pkg: &Value) -> Option<String> {
    pkg.get("style")
        .and_then(|v| v.as_str())
        .map(|s| strip_dot_slash(s).to_string())
}

fn strip_dot_slash(s: &str) -> &str {
    s.strip_prefix("./").unwrap_or(s)
}
