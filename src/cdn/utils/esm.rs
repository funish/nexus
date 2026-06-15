use anyhow::Result;
use rolldown::{Bundler, BundlerOptions, InputItem, OutputFormat, Platform};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::storage::SharedStorage;

#[derive(Clone)]
pub struct EsmBundleOptions {
    pub package_name: String,
    pub version: String,
    pub entry_point: String,
}

pub async fn bundle_esm_package(
    storage: &SharedStorage,
    options: &EsmBundleOptions,
) -> Result<String> {
    let cache_base = format!("cdn/npm/{}/{}", options.package_name, options.version);
    let esm_key = format!("{cache_base}/+esm");

    if let Some(cached) = storage.get_raw(&esm_key).await {
        return String::from_utf8(cached)
            .map_err(|e| anyhow::anyhow!("cached bundle is not valid UTF-8: {e}"));
    }

    // Single-flight: concurrent +esm requests for the same package share one
    // bundling run; followers re-read the cached bundle afterward.
    let storage_for_fn = storage.clone();
    let opts = options.clone();
    let key = esm_key.clone();
    super::singleflight::run_once(&esm_key, || {
        let storage = storage_for_fn.clone();
        let opts = opts.clone();
        let key = key.clone();
        async move {
            if storage.get_raw(&key).await.is_some() {
                return;
            }
            match build_bundle(&storage, &opts).await {
                Ok(code) => storage.set_raw(&key, code.as_bytes()).await,
                Err(e) => tracing::warn!(
                    "ESM bundle failed for {}@{}: {e}",
                    opts.package_name,
                    opts.version
                ),
            }
        }
    })
    .await;

    storage
        .get_raw(&esm_key)
        .await
        .map(|d| {
            String::from_utf8(d)
                .map_err(|e| anyhow::anyhow!("cached bundle is not valid UTF-8: {e}"))
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ESM bundle unavailable for {}@{}",
                options.package_name,
                options.version
            )
        })?
}

async fn build_bundle(storage: &SharedStorage, options: &EsmBundleOptions) -> Result<String> {
    let cache_base = format!("cdn/npm/{}/{}", options.package_name, options.version);

    let meta = storage.get_meta(&cache_base).await.ok_or_else(|| {
        anyhow::anyhow!(
            "Package {}@{} is not cached yet",
            options.package_name,
            options.version
        )
    })?;

    if meta.files.is_none() {
        anyhow::bail!(
            "Package {}@{} is not cached yet",
            options.package_name,
            options.version
        );
    }

    // Read package.json for dependencies
    let pkg_json_key = format!("{cache_base}/package.json");
    let pkg_json_data = storage.get_raw(&pkg_json_key).await.ok_or_else(|| {
        anyhow::anyhow!(
            "package.json not found for {}@{}",
            options.package_name,
            options.version
        )
    })?;
    let pkg_json: serde_json::Value = serde_json::from_slice(&pkg_json_data)?;

    // Collect external dependency names
    let mut externals: Vec<String> = Vec::new();
    for field in &["dependencies", "peerDependencies"] {
        if let Some(deps) = pkg_json[*field].as_object() {
            externals.extend(deps.keys().cloned());
        }
    }

    // Resolve dependency versions for import rewriting: fetch each dep's
    // metadata and pick the newest published version satisfying the declared
    // range (esm.sh behavior). The old min-version floor picked stale releases
    // (e.g. 1.0.0 for ^1.2.0). Fetches run concurrently and hit the metadata
    // cache, so repeated bundles of the same dep tree are cheap.
    let mut tasks = tokio::task::JoinSet::new();
    for field in &["dependencies", "peerDependencies"] {
        if let Some(deps) = pkg_json[*field].as_object() {
            for (name, range) in deps {
                if let Some(range_str) = range.as_str()
                    && let Ok(req) = range_str.parse::<node_semver::Range>()
                {
                    let storage = storage.clone();
                    let name = name.clone();
                    tasks.spawn(async move {
                        let v = latest_version_satisfying(&storage, &name, &req).await;
                        (name, v)
                    });
                }
            }
        }
    }
    let mut dep_versions: HashMap<String, String> = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok((name, Some(v))) = res {
            dep_versions.insert(name, v);
        }
    }

    // Extract all cached files to temp directory
    let files = meta.files.unwrap_or_default();
    let tmp_dir = tempfile::tempdir()?;
    let tmp_path = tmp_dir.path();

    for file in &files {
        let cache_key = format!("{cache_base}/{}", file.name);
        if let Some(data) = storage.get_raw(&cache_key).await {
            let file_path = tmp_path.join(&file.name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file_path, data)?;
        }
    }

    let entry = options
        .entry_point
        .strip_prefix("./")
        .unwrap_or(&options.entry_point);
    // Use the declared entry when present; otherwise fall back to common entry
    // filenames that actually exist in the extracted package.
    let entry_path = tmp_path.join(entry);
    let entry_path = if entry_path.exists() {
        entry_path
    } else {
        crate::cdn::utils::entry::ENTRY_FALLBACKS
            .iter()
            .map(|c| tmp_path.join(c))
            .find(|p| p.exists())
            .ok_or_else(|| anyhow::anyhow!("Entry point {entry} not found in extracted files"))?
    };

    // Configure rolldown bundler
    let out_dir = tmp_path.join("__out__");
    std::fs::create_dir_all(&out_dir)?;

    let bundler_options = BundlerOptions {
        input: Some(vec![InputItem {
            name: Some("entry".to_string()),
            import: entry_path.to_string_lossy().to_string(),
        }]),
        cwd: Some(PathBuf::from(tmp_path)),
        dir: Some(out_dir.to_string_lossy().to_string()),
        format: Some(OutputFormat::Esm),
        platform: Some(Platform::Browser),
        external: Some(externals.into()),
        minify: Some(rolldown::RawMinifyOptions::Bool(true)),
        ..Default::default()
    };

    // Run bundler
    let mut bundler = Bundler::new(bundler_options)?;
    let _output = bundler.write().await?;

    // Read output file from disk
    let mut code = String::new();
    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "mjs" || e == "js") {
                code = std::fs::read_to_string(&path)?;
                break;
            }
        }
    }

    if code.is_empty() {
        anyhow::bail!("Rolldown produced no output");
    }

    // Rewrite bare imports to CDN paths
    code = rewrite_imports(&code, &dep_versions);

    // Clean up
    bundler.close().await?;

    Ok(code)
}

fn rewrite_imports(code: &str, deps: &HashMap<String, String>) -> String {
    let import_re =
        regex::Regex::new(r#"(?:import|export)\*?(?:\s*[\s\S]*?from\s*|)["']([^"']+)["']"#)
            .unwrap();
    let dynamic_re = regex::Regex::new(r#"import\s*\(\s*["']([^"']+)"#).unwrap();

    let mut result = code.to_string();

    // Rewrite static imports
    result = import_re
        .replace_all(&result, |caps: &regex::Captures| {
            let full = &caps[0];
            let spec = &caps[1];

            if spec.starts_with('.') || spec.starts_with('/') {
                return full.to_string();
            }

            let cdn_path = to_cdn_path(spec, deps);
            full.replace(spec, &cdn_path)
        })
        .to_string();

    // Rewrite dynamic imports
    result = dynamic_re
        .replace_all(&result, |caps: &regex::Captures| {
            let full = &caps[0];
            let spec = &caps[1];

            if spec.starts_with('.') || spec.starts_with('/') {
                return full.to_string();
            }

            let cdn_path = to_cdn_path(spec, deps);
            full.replace(spec, &cdn_path)
        })
        .to_string();

    result
}

fn to_cdn_path(spec: &str, deps: &HashMap<String, String>) -> String {
    let parts: Vec<&str> = spec.split('/').collect();

    let (dep_name, sub_path) = if parts.first().is_some_and(|p| p.starts_with('@')) {
        let name = if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            parts[0].to_string()
        };
        let sub = parts[2..].join("/");
        (name, sub)
    } else {
        let name = parts.first().unwrap_or(&"").to_string();
        let sub = parts[1..].join("/");
        (name, sub)
    };

    if let Some(version) = deps.get(&dep_name) {
        if sub_path.is_empty() {
            format!("/cdn/npm/{dep_name}@{version}/+esm")
        } else {
            format!("/cdn/npm/{dep_name}@{version}/{sub_path}")
        }
    } else if sub_path.is_empty() {
        format!("/cdn/npm/{dep_name}/+esm")
    } else {
        format!("/cdn/npm/{dep_name}/{sub_path}")
    }
}

/// Newest published version of `package_name` satisfying `req`, or `None` if
/// metadata is unavailable or no version matches. Returning `None` (rather than
/// erroring) keeps bundling resilient to a registry hiccup — the caller just
/// leaves that import bare.
async fn latest_version_satisfying(
    storage: &SharedStorage,
    package_name: &str,
    req: &node_semver::Range,
) -> Option<String> {
    let metadata = crate::cdn::utils::registry::fetch_npm_metadata(storage, package_name)
        .await
        .ok()?;
    let versions = metadata.get("versions")?.as_object()?;
    versions
        .keys()
        .filter_map(|s| s.parse::<node_semver::Version>().ok())
        .filter(|v| v.satisfies(req))
        .max()
        .map(|v| v.to_string())
}
