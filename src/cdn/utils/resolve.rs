use node_semver::{Range, Version};
use serde_json::Value;

pub struct ResolvedVersion {
    pub version: String,
    pub version_info: Value,
}

pub fn resolve_registry_version(metadata: &Value, requested: &str) -> Option<ResolvedVersion> {
    // 1. Exact match
    if let Some(info) = metadata["versions"].get(requested) {
        return Some(ResolvedVersion {
            version: requested.to_string(),
            version_info: info.clone(),
        });
    }

    // 2. Semver range match (npm flavor via node-semver: bare "3"/"3.7", "^", "~", ...)
    if let Ok(range) = requested.parse::<Range>() {
        let versions: Vec<&str> = metadata["versions"]
            .as_object()
            .map(|obj| obj.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        if let Some(max) = versions
            .iter()
            .filter_map(|v| v.parse::<Version>().ok())
            .filter(|v| v.satisfies(&range))
            .max()
        {
            let ver_str = max.to_string();
            if let Some(info) = metadata["versions"].get(&ver_str) {
                return Some(ResolvedVersion {
                    version: ver_str,
                    version_info: info.clone(),
                });
            }
        }
    }

    // 3. Fallback to dist-tags.latest
    let latest = metadata["dist-tags"]["latest"].as_str()?;
    let info = metadata["versions"].get(latest)?;
    Some(ResolvedVersion {
        version: latest.to_string(),
        version_info: info.clone(),
    })
}

/// All versions satisfying a semver range, newest first. Empty when `requested`
/// is not a range (exact version / dist-tag). Used for jsDelivr-style version
/// fallback: when the newest matching version lacks a file, try older ones.
pub fn resolve_registry_versions_desc(metadata: &Value, requested: &str) -> Vec<String> {
    let Ok(range) = requested.parse::<Range>() else {
        return Vec::new();
    };
    let mut versions: Vec<Version> = metadata["versions"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, _)| k.parse::<Version>().ok())
        .filter(|v| v.satisfies(&range))
        .collect();
    versions.sort_by(|a, b| b.cmp(a));
    versions.into_iter().map(|v| v.to_string()).collect()
}

pub fn resolve_from_tags(tags: &[String], requested: &str) -> Option<String> {
    // Exact match (mirrors allVersions.includes(version)).
    if tags.iter().any(|t| t == requested) {
        return Some(requested.to_string());
    }

    // Semver range (npm flavor; handles "v"-prefixed tags natively).
    //
    // "*", "^5", "~5.3" land here. A bare branch/tag name such as "main" or "dev"
    // fails to parse as a Range, so we return None and the caller keeps the name
    // as-is — matching gh/[...path].ts, which only resolves to the latest tag when
    // the caller passes no version at all. Falling back to the latest tag here would
    // turn a `@main` request into the newest release.
    if let Ok(range) = requested.parse::<Range>()
        && let Some(max) = tags
            .iter()
            .filter_map(|t| t.parse::<Version>().ok())
            .filter(|v| v.satisfies(&range))
            .max()
    {
        return Some(max.to_string());
    }

    None
}
