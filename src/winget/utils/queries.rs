//! index.db queries: build the search index and look up packages/versions.

use rusqlite::{Connection, params};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use super::response::ManifestVersion;
use super::search::WinGetSearchEntry;

const DELIM: &str = "\x1E";

/// Compiled once: coerce_semver is called per version-comparison during sort,
/// so a per-call `Regex::new` dominated build time (millions of recompiles).
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?").unwrap());

/// Build the unified search index (mirrors search.ts buildSearchIndex).
/// Fetches all fields in one query so search responses need no second DB hit.
///
/// Six CTEs pre-aggregate every multi-valued field once, all per package `id` —
/// the main scan JOINs them instead of re-running a correlated subquery per row.
/// Since every field is id-scoped, each entry is complete on its first row, so no
/// per-row accumulation is needed in Rust.
pub fn build_search_index(conn: &Connection) -> anyhow::Result<Vec<WinGetSearchEntry>> {
    let sql = r#"
    WITH
    id_monikers AS (
      SELECT m.id AS k, GROUP_CONCAT(mk.moniker, ?1) AS v FROM (
        SELECT DISTINCT id, moniker FROM manifest
      ) m JOIN monikers mk ON mk.rowid = m.moniker WHERE mk.moniker != '' GROUP BY m.id
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
    )
    SELECT DISTINCT i.id, n.name, np.norm_publisher,
      im.v AS monikers, it.v AS tags, ic.v AS commands,
      v.version, ch.channel,
      ipf.v AS pfns, ipc.v AS productcodes, iuc.v AS upgradecodes
    FROM manifest m
    JOIN ids i ON m.id = i.rowid
    JOIN names n ON m.name = n.rowid
    JOIN versions v ON m.version = v.rowid
    LEFT JOIN channels ch ON m.channel = ch.rowid
    LEFT JOIN norm_publishers_map npm ON npm.manifest = m.rowid
    LEFT JOIN norm_publishers np ON np.rowid = npm.norm_publisher
    LEFT JOIN id_monikers im ON im.k = m.id
    LEFT JOIN id_tags it ON it.k = m.id
    LEFT JOIN id_commands ic ON ic.k = m.id
    LEFT JOIN id_pfns ipf ON ipf.k = m.id
    LEFT JOIN id_productcodes ipc ON ipc.k = m.id
    LEFT JOIN id_upgradecodes iuc ON iuc.k = m.id
    "#;

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![DELIM], |row| {
        Ok(RowData {
            id: row.get(0)?,
            name: row.get(1)?,
            norm_publisher: row.get::<_, Option<String>>(2)?,
            monikers: row.get::<_, Option<String>>(3)?,
            tags: row.get::<_, Option<String>>(4)?,
            commands: row.get::<_, Option<String>>(5)?,
            version: row.get(6)?,
            channel: row.get::<_, Option<String>>(7)?,
            pfns: row.get::<_, Option<String>>(8)?,
            productcodes: row.get::<_, Option<String>>(9)?,
            upgradecodes: row.get::<_, Option<String>>(10)?,
        })
    })?;

    let mut entry_map: HashMap<String, WinGetSearchEntry> = HashMap::new();
    for row in rows {
        let r = row?;
        let entry = entry_map.entry(r.id.clone()).or_insert_with(|| {
            let publisher = r
                .norm_publisher
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| r.id.split('.').next().unwrap_or("").to_string());
            // Every multi-valued field is aggregated by id in the CTEs, so the first
            // row already carries the complete value — no per-row accumulation.
            WinGetSearchEntry {
                id: r.id.clone(),
                name: r.name.clone(),
                publisher,
                monikers: split_delim(&r.monikers),
                tags: split_delim(&r.tags),
                commands: split_delim(&r.commands),
                package_family_names: split_delim(&r.pfns),
                product_codes: split_delim(&r.productcodes),
                upgrade_codes: split_delim(&r.upgradecodes),
                versions: Vec::new(),
            }
        });

        // Deduplicate versions (version + channel).
        let already = entry
            .versions
            .iter()
            .any(|v| v.package_version == r.version && v.channel == r.channel);
        if !already {
            entry.versions.push(ManifestVersion {
                package_version: r.version.clone(),
                channel: r.channel.clone().filter(|c| !c.is_empty()),
            });
        }
    }

    let mut entries: Vec<WinGetSearchEntry> = entry_map.into_values().collect();
    for e in &mut entries {
        e.versions
            .sort_by(|a, b| compare_version(&b.package_version, &a.package_version));
    }
    Ok(entries)
}

struct RowData {
    id: String,
    name: String,
    norm_publisher: Option<String>,
    monikers: Option<String>,
    tags: Option<String>,
    commands: Option<String>,
    version: String,
    channel: Option<String>,
    pfns: Option<String>,
    productcodes: Option<String>,
    upgradecodes: Option<String>,
}

/// Split on DELIM and deduplicate, preserving first-seen order.
fn split_delim(s: &Option<String>) -> Vec<String> {
    match s {
        Some(s) if !s.is_empty() => {
            let mut seen: HashSet<&str> = HashSet::new();
            s.split(DELIM)
                .filter(|x| !x.is_empty())
                .filter(|x| seen.insert(*x))
                .map(|x| x.to_string())
                .collect()
        }
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
        let conn = crate::winget::utils::db::open_db(path).expect("open_db");
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
