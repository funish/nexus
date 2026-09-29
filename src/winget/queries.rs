//! index.db queries: build the search index and look up packages/versions.

use rusqlite::{Connection, params};
use std::sync::LazyLock;

use super::rest::ManifestVersion;
use super::search::WinGetSearchEntry;

const DELIM: &str = "\x1E";
const VERSION_DELIM: &str = "\x1F";

/// Compiled once: sorting builds one key per version, so repeated regex
/// compilation would otherwise dominate index construction.
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?").unwrap());

/// Build the unified search index (mirrors search.ts buildSearchIndex).
/// Aggregates multi-valued fields and versions by package id in SQLite so the
/// Rust-side scan receives one row per package rather than one per manifest.
pub fn build_search_index(conn: &Connection) -> anyhow::Result<Vec<WinGetSearchEntry>> {
    let sql = r#"
    WITH
    id_names AS (
      SELECT k, GROUP_CONCAT(name, ?1) AS v FROM (
        SELECT DISTINCT id AS k, name FROM manifest
      ) GROUP BY k
    ),
    id_publishers AS (
      SELECT k, GROUP_CONCAT(norm_publisher, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, np.norm_publisher FROM manifest mm
        JOIN norm_publishers_map npm ON npm.manifest = mm.rowid
        JOIN norm_publishers np ON np.rowid = npm.norm_publisher
      ) GROUP BY k
    ),
    id_monikers AS (
      SELECT k, GROUP_CONCAT(m.moniker, ?1) AS v FROM (
        SELECT DISTINCT id AS k, moniker FROM manifest
      ) m JOIN monikers mk ON mk.rowid = m.moniker WHERE mk.moniker != '' GROUP BY k
    ),
    id_tags AS (
      SELECT k, GROUP_CONCAT(tag, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, t.tag FROM manifest mm
        JOIN tags_map tm ON tm.manifest = mm.rowid JOIN tags t ON t.rowid = tm.tag
      ) GROUP BY k
    ),
    id_commands AS (
      SELECT k, GROUP_CONCAT(command, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, c.command FROM manifest mm
        JOIN commands_map cm ON cm.manifest = mm.rowid JOIN commands c ON c.rowid = cm.command
      ) GROUP BY k
    ),
    id_pfns AS (
      SELECT k, GROUP_CONCAT(pfn, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, p.pfn FROM manifest mm
        JOIN pfns_map pm ON pm.manifest = mm.rowid JOIN pfns p ON p.rowid = pm.pfn
      ) GROUP BY k
    ),
    id_productcodes AS (
      SELECT k, GROUP_CONCAT(productcode, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, pc.productcode FROM manifest mm
        JOIN productcodes_map pcm ON pcm.manifest = mm.rowid JOIN productcodes pc ON pc.rowid = pcm.productcode
      ) GROUP BY k
    ),
    id_upgradecodes AS (
      SELECT k, GROUP_CONCAT(upgradecode, ?1) AS v FROM (
        SELECT DISTINCT mm.id AS k, uc.upgradecode FROM manifest mm
        JOIN upgradecodes_map ucm ON ucm.manifest = mm.rowid JOIN upgradecodes uc ON uc.rowid = ucm.upgradecode
      ) GROUP BY k
    ),
    id_version_rows AS (
      SELECT DISTINCT
        m.id AS k,
        v.version || char(31) || COALESCE(ch.channel, '') AS entry
      FROM manifest m
      JOIN versions v ON v.rowid = m.version
      LEFT JOIN channels ch ON ch.rowid = m.channel
    ),
    id_versions AS (
      SELECT k, GROUP_CONCAT(entry, ?1) AS v FROM id_version_rows GROUP BY k
    )
    SELECT i.id, n.v AS names, ip.v AS norm_publishers,
      im.v AS monikers, it.v AS tags, ic.v AS commands,
      iv.v AS versions,
      ipf.v AS pfns, ipc.v AS productcodes, iuc.v AS upgradecodes
    FROM ids i
    JOIN id_names n ON n.k = i.rowid
    LEFT JOIN id_publishers ip ON ip.k = i.rowid
    LEFT JOIN id_monikers im ON im.k = i.rowid
    LEFT JOIN id_tags it ON it.k = i.rowid
    LEFT JOIN id_commands ic ON ic.k = i.rowid
    LEFT JOIN id_versions iv ON iv.k = i.rowid
    LEFT JOIN id_pfns ipf ON ipf.k = i.rowid
    LEFT JOIN id_productcodes ipc ON ipc.k = i.rowid
    LEFT JOIN id_upgradecodes iuc ON iuc.k = i.rowid
    "#;

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![DELIM], |row| {
        Ok(RowData {
            id: row.get(0)?,
            names: row.get::<_, Option<String>>(1)?,
            norm_publishers: row.get::<_, Option<String>>(2)?,
            monikers: row.get::<_, Option<String>>(3)?,
            tags: row.get::<_, Option<String>>(4)?,
            commands: row.get::<_, Option<String>>(5)?,
            versions: row.get::<_, Option<String>>(6)?,
            pfns: row.get::<_, Option<String>>(7)?,
            productcodes: row.get::<_, Option<String>>(8)?,
            upgradecodes: row.get::<_, Option<String>>(9)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        let r = row?;
        let name = split_delim(&r.names).into_iter().next().unwrap_or_default();
        let publisher = split_delim(&r.norm_publishers)
            .into_iter()
            .next()
            .unwrap_or_else(|| r.id.split('.').next().unwrap_or("").to_string());
        entries.push(WinGetSearchEntry {
            id: r.id,
            name,
            publisher,
            monikers: split_delim(&r.monikers),
            tags: split_delim(&r.tags),
            commands: split_delim(&r.commands),
            package_family_names: split_delim(&r.pfns),
            product_codes: split_delim(&r.productcodes),
            upgrade_codes: split_delim(&r.upgradecodes),
            versions: parse_versions(&r.versions),
        });
    }

    for entry in &mut entries {
        let mut versions = entry
            .versions
            .drain(..)
            .map(|version| (version_sort_key(&version.package_version), version))
            .collect::<Vec<_>>();
        versions.sort_by(|a, b| compare_version_keys(&b.0, &a.0));
        entry.versions = versions.into_iter().map(|(_, version)| version).collect();
    }
    Ok(entries)
}

struct RowData {
    id: String,
    names: Option<String>,
    norm_publishers: Option<String>,
    monikers: Option<String>,
    tags: Option<String>,
    commands: Option<String>,
    versions: Option<String>,
    pfns: Option<String>,
    productcodes: Option<String>,
    upgradecodes: Option<String>,
}

/// Decode the SQLite-aggregated `version || channel` entries.
fn parse_versions(s: &Option<String>) -> Vec<ManifestVersion> {
    let Some(s) = s else {
        return Vec::new();
    };
    s.split(DELIM)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let (version, channel) = entry.split_once(VERSION_DELIM).unwrap_or((entry, ""));
            (!version.is_empty()).then_some(ManifestVersion {
                package_version: version.to_string(),
                channel: (!channel.is_empty()).then(|| channel.to_string()),
            })
        })
        .collect()
}

/// Split SQLite's aggregated values. Each SQL CTE already deduplicates entries.
fn split_delim(s: &Option<String>) -> Vec<String> {
    match s {
        Some(s) if !s.is_empty() => s
            .split(DELIM)
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// Compare versions (mirrors version.ts: semver coerce first, fall back to per-segment numeric).
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    if let (Some(sa), Some(sb)) = (coerce_semver(a), coerce_semver(b)) {
        return sa.cmp(&sb);
    }
    let pa: Vec<u64> = a.split('.').filter_map(|x| x.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|x| x.parse().ok()).collect();
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

type VersionSortKey = (Option<semver::Version>, Vec<u64>);

fn version_sort_key(v: &str) -> VersionSortKey {
    (
        coerce_semver(v),
        v.split('.').filter_map(|x| x.parse().ok()).collect(),
    )
}

fn compare_version_keys(a: &VersionSortKey, b: &VersionSortKey) -> std::cmp::Ordering {
    if let (Some(sa), Some(sb)) = (&a.0, &b.0) {
        return sa.cmp(sb);
    }
    let (pa, pb) = (&a.1, &b.1);
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

/// Extract the first x.y.z from a string as a semver Version (mirrors semver.coerce).
fn coerce_semver(v: &str) -> Option<semver::Version> {
    let caps = SEMVER_RE.captures(v)?;
    let major: u64 = caps[1].parse().ok()?;
    let minor: u64 = caps
        .get(2)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let patch: u64 = caps
        .get(3)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    Some(semver::Version::new(major, minor, patch))
}

/// Whether a package exists (EXISTS query).
pub fn package_exists(conn: &Connection, package_id: &str) -> anyhow::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM manifest m JOIN ids i ON m.id = i.rowid WHERE i.id = ?1)",
        params![package_id],
        |r| r.get(0),
    )?;
    Ok(exists)
}

/// All versions of a package (descending).
pub fn get_package_versions(conn: &Connection, package_id: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT v.version FROM manifest m JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid WHERE i.id = ?1",
    )?;
    let mut versions: Vec<String> = stmt
        .query_map(params![package_id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    versions.sort_by(|a, b| compare_version(b, a));
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validates the full winget DB layer against a real index.db extracted from
    /// source.msix (dev-only). Skipped when the file is absent so CI stays hermetic.
    #[test]
    fn real_index_db_roundtrip() {
        let path = "E:/nexus/.temp/winget/Public/index.db";
        if !std::path::Path::new(path).exists() {
            eprintln!("skipped: {path} not present");
            return;
        }
        let t_open = std::time::Instant::now();
        let conn = Connection::open(path).expect("open index.db");
        let _ = conn.busy_timeout(std::time::Duration::from_secs(30));
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS tags_map_manifest_idx ON tags_map(manifest);
             CREATE INDEX IF NOT EXISTS commands_map_manifest_idx ON commands_map(manifest);",
        )
        .expect("initialize query indexes");
        eprintln!("[timing] open_db: {:.3}s", t_open.elapsed().as_secs_f64());
        let t0 = std::time::Instant::now();
        let index = build_search_index(&conn).expect("build_search_index");
        eprintln!(
            "[timing] build_search_index: {:.3}s",
            t0.elapsed().as_secs_f64()
        );
        assert!(!index.is_empty(), "search index should not be empty");
        eprintln!("search index entries: {}", index.len());

        assert!(
            package_exists(&conn, "Git.Git").expect("package_exists"),
            "Git.Git should exist"
        );
        let versions = get_package_versions(&conn, "Git.Git").expect("get_package_versions");
        eprintln!("Git.Git versions: {:?}", versions);
        assert!(!versions.is_empty(), "Git.Git should have versions");
    }
}
