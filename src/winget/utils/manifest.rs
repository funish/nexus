//! GitHub raw YAML manifest fetching and version-manifest assembly (mirrors winget/manifest.ts).
//!
//! Constructs manifest paths under the winget-pkgs layout, fetches immutable
//! manifest files (cached forever), discovers a version's manifest files via the
//! tree API, and merges main/installer/locale manifests into a version manifest.

use std::time::Duration;

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

use crate::storage::SharedStorage;

use super::constants::*;
use super::response::VersionManifest;
use super::tree::{get_github_tree_paths, get_letter_directory_shas};

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
    let cache_key = format!("{WINGET_CACHE_PREFIX}/files/{manifest_path}");

    if let Some(cached) = storage.get_raw(&cache_key).await
        && let Ok(s) = String::from_utf8(cached)
    {
        return Ok(s);
    }

    let url = format!("{WINGET_GITHUB_RAW_BASE}/{manifest_path}");
    let resp = crate::http::HTTP_CLIENT
        .get(&url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch manifest: {}", resp.status());
    }

    let content = resp.text().await?;
    storage.set_raw(&cache_key, content.as_bytes()).await;
    Ok(content)
}

/// Parse YAML content into a JSON value (confbox parseYAML equivalent via serde_yml).
pub fn parse_yaml(content: &str) -> Result<Value> {
    Ok(serde_yml::from_str::<Value>(content)?)
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

    let paths = get_github_tree_paths(storage, sha, &format!("manifests/{letter}")).await?;

    let publisher = parts[0];
    let name = parts[1..].join("/");
    let prefix = format!("{publisher}/{name}/{version}/");

    Ok(paths
        .iter()
        .filter(|p| p.starts_with(&prefix) && p.ends_with(".yaml"))
        .map(|p| format!("manifests/{letter}/{p}"))
        .collect())
}

/// Assemble a merged version manifest from all manifest files (mirrors buildVersionManifest).
pub async fn build_version_manifest(
    storage: &SharedStorage,
    package_id: &str,
    version: &str,
) -> Result<Option<VersionManifest>> {
    let files = get_version_manifests(storage, package_id, version).await?;
    if files.is_empty() {
        return Ok(None);
    }

    let mut entry = VersionManifest {
        package_version: version.to_string(),
        default_locale: None,
        channel: None,
        locales: None,
        installers: None,
    };

    for path in &files {
        let filename = path.rsplit('/').next().unwrap_or("");
        let content = match fetch_manifest_content(storage, path).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let manifest = match parse_yaml(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

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
            let has_default_locale_file = default_locale
                .as_ref()
                .map(|dl| {
                    files
                        .iter()
                        .any(|p| p.contains(&format!(".locale.{dl}.yaml")))
                })
                .unwrap_or(false);

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
        } else if LOCALE_FILE_RE.is_match(filename) {
            entry.locales.get_or_insert_with(Vec::new).push(manifest);
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
