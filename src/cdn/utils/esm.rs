use anyhow::Result;
use rolldown::{Bundler, BundlerOptions, InputItem, OutputFormat, Platform};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::storage::SharedStorage;

// Bare-import specifiers to rewrite to CDN paths. Compiled once, reused per bundle.
static IMPORT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?:import|export)\*?(?:\s*[\s\S]*?from\s*|)["']([^"']+)["']"#).unwrap()
});
static DYNAMIC_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"import\s*\(\s*["']([^"']+)"#).unwrap());

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
    // Cap concurrent bundles: rolldown is CPU/memory-heavy and each bundle
    // unpacks the package to a temp dir. Without this a cold-start burst of
    // distinct packages can OOM the process.
    let _bundle_permit = super::concurrency::BUNDLE_SEMAPHORE
        .acquire()
        .await
        .unwrap();

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

    // Collect dependency entries once: externals are their names, and each entry
    // with a resolvable range also drives a concurrent version fetch for import
    // rewriting (esm.sh behavior — newest published version satisfying the range).
    // Fetches hit the metadata cache, so repeated bundles of the same dep tree
    // are cheap.
    let dep_entries: Vec<(String, Option<String>)> = ["dependencies", "peerDependencies"]
        .iter()
        .filter_map(|f| pkg_json[*f].as_object())
        .flatten()
        .map(|(name, range)| (name.clone(), range.as_str().map(String::from)))
        .collect();
    let externals: Vec<String> = dep_entries.iter().map(|(name, _)| name.clone()).collect();

    let mut tasks = tokio::task::JoinSet::new();
    for (name, range_str) in &dep_entries {
        if let Some(rs) = range_str
            && let Ok(req) = rs.parse::<node_semver::Range>()
        {
            let storage = storage.clone();
            let name = name.clone();
            tasks.spawn(async move {
                let v = latest_version_satisfying(&storage, &name, &req).await;
                (name, v)
            });
        }
    }
    let mut dep_versions: HashMap<String, String> = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok((name, Some(v))) = res {
            dep_versions.insert(name, v);
        }
    }

    // Read every cached file into memory first (async), so the blocking extract step
    // never touches the async storage layer.
    let files = meta.files.unwrap_or_default();
    let mut file_bytes: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    for file in &files {
        let cache_key = format!("{cache_base}/{}", file.name);
        if let Some(data) = storage.get_raw(&cache_key).await {
            file_bytes.push((file.name.clone(), data));
        }
    }

    let entry = options
        .entry_point
        .strip_prefix("./")
        .unwrap_or(&options.entry_point)
        .to_string();
    let entry_fallbacks: Vec<String> = crate::cdn::utils::entry::ENTRY_FALLBACKS
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    // BLOCKING: write all files to a temp dir and resolve the entry path. std::fs is
    // synchronous — off the async worker thread so it never stalls a request.
    // `_tmp_dir` keeps the TempDir alive (its Drop deletes the dir) for the whole
    // bundling step — naming it `_tmp_dir` rather than `_` preserves it until scope end.
    let (_tmp_dir, tmp_path_buf, entry_path) =
        tokio::task::spawn_blocking(move || -> Result<(tempfile::TempDir, PathBuf, PathBuf)> {
            let tmp_dir = tempfile::tempdir()?;
            let tmp_path_buf = tmp_dir.path().to_path_buf();

            for (name, data) in &file_bytes {
                let file_path = tmp_dir.path().join(name);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file_path, data)?;
            }

            // Use the declared entry when present; otherwise fall back to common entry
            // filenames that actually exist in the extracted package.
            let entry_path = tmp_dir.path().join(&entry);
            let entry_path = if entry_path.exists() {
                entry_path
            } else {
                entry_fallbacks
                    .iter()
                    .map(|c| tmp_dir.path().join(c))
                    .find(|p| p.exists())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Entry point {entry} not found in extracted files")
                    })?
            };
            Ok((tmp_dir, tmp_path_buf, entry_path))
        })
        .await
        .map_err(|e| anyhow::anyhow!("extract task panicked: {e}"))??;

    let tmp_path = tmp_path_buf.as_path();

    // Configure rolldown bundler
    let out_dir = tmp_path.join("__out__");

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

    // Run bundler (rolldown's own async API — CPU-heavy but correctly on the runtime).
    let mut bundler = Bundler::new(bundler_options)?;
    let _output = bundler.write().await?;

    // BLOCKING: read the output file off the async worker.
    let out_dir_clone = out_dir.clone();
    let code = tokio::task::spawn_blocking(move || -> Result<String> {
        if let Ok(entries) = std::fs::read_dir(&out_dir_clone) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "mjs" || e == "js") {
                    return Ok(std::fs::read_to_string(&path)?);
                }
            }
        }
        anyhow::bail!("Rolldown produced no output")
    })
    .await
    .map_err(|e| anyhow::anyhow!("output read task panicked: {e}"))??;

    // Rewrite bare imports to CDN paths
    let code = rewrite_imports(&code, &dep_versions);

    // Clean up
    bundler.close().await?;

    Ok(code)
}

/// Rewrite bare import specifiers to CDN paths. Chains both regex passes into a
/// single owning String instead of copying the (potentially hundreds-of-KB) bundle
/// three times — `replace_all` returns a `Cow`, which we own in place before the
/// next pass borrows it.
fn rewrite_imports(code: &str, deps: &HashMap<String, String>) -> String {
    // Rewrite static imports
    let owned = IMPORT_RE.replace_all(code, |caps: &regex::Captures| {
        let full = &caps[0];
        let spec = &caps[1];
        if spec.starts_with('.') || spec.starts_with('/') {
            return full.to_string();
        }
        let cdn_path = to_cdn_path(spec, deps);
        full.replace(spec, &cdn_path)
    });

    // Rewrite dynamic imports — borrow the already-owned string from pass 1.
    let owned = DYNAMIC_RE.replace_all(&owned, |caps: &regex::Captures| {
        let full = &caps[0];
        let spec = &caps[1];
        if spec.starts_with('.') || spec.starts_with('/') {
            return full.to_string();
        }
        let cdn_path = to_cdn_path(spec, deps);
        full.replace(spec, &cdn_path)
    });

    owned.into_owned()
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
