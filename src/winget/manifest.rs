//! GitHub raw YAML manifest fetching and version-manifest assembly (mirrors winget/manifest.ts).
//!
//! Constructs manifest paths under the winget-pkgs layout, fetches immutable
//! manifest files (cached forever), discovers a version's manifest files via the
//! tree API, and merges main/installer/locale manifests into a version manifest.

use std::time::Duration;

use anyhow::Result;
use futures::future;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

use crate::cdn::constants::CDN_MAX_PACKAGE_SIZE;
use crate::storage::SharedStorage;
use crate::utils::cache::{cache_fresh, set_mtime};
use crate::utils::concurrency::DOWNLOAD_SEMAPHORE;
use crate::utils::singleflight::run_once;

use super::constants::*;
use super::rest::VersionManifest;
use super::tree::{get_github_tree_entries, get_github_tree_paths, get_letter_directory_shas};

static LOCALE_FILE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.locale\.[^.]+\.yaml$").unwrap());

/// Manifest file kind (mirrors the `type` argument of constructManifestPath).
/// Installer/Locale are retained to mirror the original API; the discover-by-tree
/// path is used for those in practice.
#[allow(dead_code)]
pub enum ManifestType {
    Main,
    Installer,
    Locale,
}

/// Construct the GitHub raw path for a manifest file (mirrors constructManifestPath).
pub fn construct_manifest_path(
    package_id: &str,
    version: &str,
    manifest_type: ManifestType,
    locale: Option<&str>,
) -> String {
    let parts: Vec<&str> = package_id.split('.').collect();
    let publisher = parts.first().copied().unwrap_or("");
    let name = parts[1..].join("/");
    let letter = publisher
        .chars()
        .next()
        .map(|c| c.to_ascii_lowercase())
        .unwrap_or_default();

    let filename = match manifest_type {
        ManifestType::Main => format!("{package_id}.yaml"),
        ManifestType::Installer => format!("{package_id}.installer.yaml"),
        ManifestType::Locale => format!("{package_id}.locale.{}.yaml", locale.unwrap_or("")),
    };

    format!("manifests/{letter}/{publisher}/{name}/{version}/{filename}")
}

/// Fetch a manifest file's text content (mirrors fetchManifestContent).
/// Content is immutable per path, so it is cached without a TTL.
pub async fn fetch_manifest_content(
    storage: &SharedStorage,
    manifest_path: &str,
) -> Result<String> {
    let cache_key = format!(
        "{}/files/{manifest_path}",
        crate::winget::constants::cache_prefix()
    );

    if let Some(cached) = storage.get_raw(&cache_key).await
        && let Ok(s) = String::from_utf8(cached)
    {
        return Ok(s);
    }

    // A package-manifest response fans out to several files, and different requests
    // can race on the same immutable path. Single-flight turns those misses into one
    // Raw fetch; followers re-read storage after the leader writes it.
    let storage_c = storage.clone();
    let key_c = cache_key.clone();
    let path_c = manifest_path.to_string();
    crate::utils::singleflight::run_once(&cache_key, move || async move {
        if let Some(cached) = storage_c.get_raw(&key_c).await
            && String::from_utf8(cached).is_ok()
        {
            return;
        }

        // Acquire a download slot only on a cache miss — manifest content is immutable,
        // so repeat reads hit the cache and never consume a permit. This single gate
        // bounds the concurrent fetches launched by build_version_manifest's join_all.
        let _permit = DOWNLOAD_SEMAPHORE.acquire().await.unwrap();

        let url = format!("{}/{path_c}", crate::winget::constants::github_raw_base());
        // raw.githubusercontent.com does not consume GitHub API credentials;
        // reserving the token for api.github.com avoids leaking it needlessly.
        match crate::utils::http::get_fetched_with_retry(
            &url,
            Duration::from_secs(30),
            None,
            &[],
            CDN_MAX_PACKAGE_SIZE,
        )
        .await
        {
            Ok(resp) if resp.status.is_success() => {
                if let Ok(content) = String::from_utf8(resp.body)
                    && let Err(e) = storage_c.set_raw(&key_c, content.as_bytes()).await
                {
                    tracing::warn!("Failed to cache WinGet manifest {path_c}: {e}");
                }
            }
            Ok(resp) => tracing::warn!(
                "WinGet manifest fetch failed: HTTP {} for {path_c}",
                resp.status
            ),
            Err(e) => tracing::warn!("WinGet manifest fetch failed for {path_c}: {e}"),
        }
    })
    .await;

    storage
        .get_raw(&cache_key)
        .await
        .and_then(|data| String::from_utf8(data).ok())
        .ok_or_else(|| anyhow::anyhow!("manifest cache miss after single-flight: {manifest_path}"))
}

/// Parse YAML content into a JSON value (confbox parseYAML equivalent via serde_yaml).
pub fn parse_yaml(content: &str) -> Result<Value> {
    Ok(serde_yaml::from_str::<Value>(content)?)
}

/// Discover all .yaml manifest paths for a package version (mirrors getVersionManifests).
pub async fn get_version_manifests(
    storage: &SharedStorage,
    package_id: &str,
    version: &str,
) -> Result<Vec<String>> {
    let parts: Vec<&str> = package_id.split('.').collect();
    if parts.len() < 2 {
        return Ok(vec![]);
    }
    let Some(first) = parts[0].chars().next() else {
        return Ok(vec![]);
    };
    let letter = first.to_ascii_lowercase().to_string();

    let letter_shas = get_letter_directory_shas(storage).await?;
    let Some(sha) = letter_shas.get(&letter) else {
        return Ok(vec![]);
    };

    let publisher = parts[0];
    let name = parts[1..].join("/");
    let letter_entries = get_github_tree_entries(storage, sha).await?;
    let Some(publisher_sha) = letter_entries.get(publisher) else {
        return Ok(vec![]);
    };

    let publisher_entries = get_github_tree_entries(storage, publisher_sha).await?;
    let Some(package_sha) = publisher_entries.get(name.as_str()) else {
        return Ok(vec![]);
    };

    let paths = get_github_tree_paths(storage, package_sha).await?;
    let prefix = format!("{version}/");

    Ok(paths
        .into_iter()
        .filter(|path| path.starts_with(&prefix) && path.ends_with(".yaml"))
        .map(|path| format!("manifests/{letter}/{publisher}/{name}/{path}"))
        .collect())
}

/// Assemble a merged version manifest from all manifest files (mirrors buildVersionManifest).
///
/// The merged result has a soft TTL: after expiry, the previous assembled value is
/// served while one background build re-reads immutable YAML and refreshes the cache.
pub async fn build_version_manifest(
    storage: &SharedStorage,
    package_id: &str,
    version: &str,
) -> Result<Option<VersionManifest>> {
    let cache_key = format!(
        "{}/version-manifest/{package_id}/{version}",
        crate::winget::constants::cache_prefix()
    );
    if cache_fresh(storage, &cache_key, WINGET_UPDATE_INTERVAL_SECS).await
        && let Some(bytes) = storage.get_raw(&cache_key).await
        && let Ok(entry) = serde_json::from_slice::<VersionManifest>(&bytes)
    {
        return Ok(Some(entry));
    }

    // Serve the previous assembled manifest while a single background build
    // discovers paths, reads immutable YAML, parses, and refreshes the cache.
    if let Some(bytes) = storage.get_raw(&cache_key).await
        && let Ok(stale) = serde_json::from_slice::<VersionManifest>(&bytes)
    {
        if !crate::utils::singleflight::is_pending(&cache_key) {
            let refresh = version_manifest_refresher(
                storage.clone(),
                cache_key,
                package_id.to_string(),
                version.to_string(),
            );
            tokio::spawn(refresh);
        }
        return Ok(Some(stale));
    }

    version_manifest_refresher(
        storage.clone(),
        cache_key.clone(),
        package_id.to_string(),
        version.to_string(),
    )
    .await;

    storage
        .get_raw(&cache_key)
        .await
        .and_then(|bytes| serde_json::from_slice::<VersionManifest>(&bytes).ok())
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("version manifest unavailable after single-flight"))
}

async fn version_manifest_refresher(
    storage: SharedStorage,
    cache_key: String,
    package_id: String,
    version: String,
) {
    let run_key = cache_key.clone();
    run_once(&run_key, move || async move {
        if cache_fresh(&storage, &cache_key, WINGET_UPDATE_INTERVAL_SECS).await {
            return;
        }
        if let Err(e) = rebuild_version_manifest(&storage, &package_id, &version).await {
            tracing::warn!("WinGet manifest rebuild failed for {package_id}@{version}: {e}");
        }
    })
    .await;
}

async fn rebuild_version_manifest(
    storage: &SharedStorage,
    package_id: &str,
    version: &str,
) -> Result<Option<VersionManifest>> {
    let cache_key = format!(
        "{}/version-manifest/{package_id}/{version}",
        crate::winget::constants::cache_prefix()
    );
    let files = get_version_manifests(storage, package_id, version).await?;
    if files.is_empty() {
        return Ok(None);
    }

    // Fetch + parse every manifest file concurrently (mirrors the Promise.allSettled
    // in manifest.ts). fetch_manifest_content takes a shared download slot, so
    // fan-out remains bounded instead of materializing every YAML at once.
    let fetched: Vec<Option<(String, Value)>> =
        future::join_all(files.iter().map(|path| async move {
            let filename = path.rsplit('/').next()?.to_string();
            let content = fetch_manifest_content(storage, path).await.ok()?;
            let manifest = parse_yaml(&content).ok()?;
            Some((filename, manifest))
        }))
        .await;
    // Whether at least one file was fetched — gates caching so an all-failed
    // build (raw outage or throttle) isn't pinned as an empty-shell manifest.
    let any_fetched = fetched.iter().any(|f| f.is_some());

    let mut entry = VersionManifest {
        package_version: version.to_string(),
        default_locale: None,
        channel: None,
        locales: None,
        installers: None,
    };

    for (filename, manifest) in fetched.into_iter().flatten() {
        if filename == format!("{package_id}.yaml") {
            entry.default_locale = manifest
                .get("DefaultLocale")
                .and_then(|v| v.as_str())
                .map(String::from);
            entry.channel = manifest
                .get("Channel")
                .and_then(|v| v.as_str())
                .map(String::from);

            // Inline locale data when no dedicated locale file exists.
            let has_locale_data = manifest
                .get("PackageLocale")
                .and_then(|v| v.as_str())
                .is_some()
                || manifest.get("Publisher").and_then(|v| v.as_str()).is_some()
                || manifest
                    .get("PackageName")
                    .and_then(|v| v.as_str())
                    .is_some();
            let default_locale = entry.default_locale.clone().or_else(|| {
                manifest
                    .get("PackageLocale")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            // Build the locale-file suffix once (not per file) and match the path's
            // suffix precisely — a substring `contains` could match unrelated paths.
            let has_default_locale_file = match &default_locale {
                Some(dl) => {
                    let suffix = format!(".locale.{dl}.yaml");
                    files.iter().any(|p| p.ends_with(&suffix))
                }
                None => false,
            };

            if has_locale_data && !has_default_locale_file {
                let mut locale = manifest.clone();
                if let Some(obj) = locale.as_object_mut()
                    && let Some(dl) = &default_locale
                {
                    obj.insert("PackageLocale".to_string(), Value::String(dl.clone()));
                }
                entry.locales.get_or_insert_with(Vec::new).insert(0, locale);
            }
        } else if filename.ends_with(".installer.yaml") {
            if let Some(installers) = manifest.get("Installers").and_then(|v| v.as_array()) {
                entry.installers = Some(
                    installers
                        .iter()
                        .map(|inst| merge_installer(&manifest, inst))
                        .collect(),
                );
            }
        } else if LOCALE_FILE_RE.is_match(&filename) {
            entry.locales.get_or_insert_with(Vec::new).push(manifest);
        }
    }

    // Cache only a non-degenerate result (any_fetched is false only when every
    // file fetch failed); a cache write failure is otherwise non-fatal.
    if any_fetched && let Ok(bytes) = serde_json::to_vec(&entry) {
        match storage.set_raw(&cache_key, &bytes).await {
            Ok(()) => set_mtime(storage, &cache_key).await,
            Err(e) => tracing::warn!("Failed to cache version manifest {cache_key}: {e}"),
        }
    }

    Ok(Some(entry))
}

/// Merge a manifest with a single installer entry ({ ...manifest, ...installer }).
pub fn merge_installer(manifest: &Value, installer: &Value) -> Value {
    let mut merged = manifest.clone();
    if let (Some(obj), Some(inst_obj)) = (merged.as_object_mut(), installer.as_object()) {
        for (k, v) in inst_obj {
            obj.insert(k.clone(), v.clone());
        }
    }
    merged
}
