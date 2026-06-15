//! WinGet search algorithm — replicates the @nlptools/distance FuzzySearch
//! semantics using strsim.
//!
//! The original search.ts FuzzySearch is based on levenshtein. Here we use
//! strsim::normalized_levenshtein (returns 0..1 similarity), with weighted
//! multi-key aggregation and a threshold. The weighted-score behavior matches
//! @nlptools/distance exactly — including the quirk that array fields are
//! treated as empty strings (extractKeyValue returns "" for non-strings).

use serde::{Deserialize, Serialize};
use strsim::normalized_levenshtein;

use super::response::{
    ManifestSearchResult, ManifestVersion, MatchType, PackageMatchField, PackageMatchFilter,
};
use super::token::decode_continuation_token;

/// Search index entry. Serialized only for persistence (mirrors search.ts
/// persistSearchIndex → cacheStorage); the HTTP response uses ManifestSearchResult.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinGetSearchEntry {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub monikers: Vec<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
    pub versions: Vec<ManifestVersion>,
    pub package_family_names: Vec<String>,
    pub product_codes: Vec<String>,
    pub upgrade_codes: Vec<String>,
}

/// Weighted search key weights (mirrors search.ts SEARCH_KEYS).
const WEIGHT_ID: f64 = 2.0;
const WEIGHT_NAME: f64 = 2.0;
const WEIGHT_PUBLISHER: f64 = 1.0;
const WEIGHT_MONIKERS: f64 = 1.5;
const WEIGHT_TAGS: f64 = 0.5;
const WEIGHT_COMMANDS: f64 = 1.5;
const WEIGHT_PFNS: f64 = 1.0;
const WEIGHT_PRODUCT_CODES: f64 = 1.0;
const WEIGHT_UPGRADE_CODES: f64 = 1.0;

/// Sum of all key weights — used as the normalization denominator. Even though
/// array fields contribute 0 to the score, their weights remain in the sum,
/// matching @nlptools/distance FuzzySearch.resolveKeys.
const TOTAL_WEIGHT: f64 = WEIGHT_ID
    + WEIGHT_NAME
    + WEIGHT_PUBLISHER
    + WEIGHT_MONIKERS
    + WEIGHT_TAGS
    + WEIGHT_COMMANDS
    + WEIGHT_PFNS
    + WEIGHT_PRODUCT_CODES
    + WEIGHT_UPGRADE_CODES;

/// Search result.
pub struct SearchResult {
    pub results: Vec<ManifestSearchResult>,
    pub has_more: bool,
    pub offset: usize,
}

/// Case-folded match of value against keyword by matchType (used for inclusions/filters).
fn match_string(value: &str, keyword: &str, match_type: MatchType) -> bool {
    let lv = value.to_lowercase();
    let lk = keyword.to_lowercase();
    match match_type {
        MatchType::Exact => lv == lk,
        MatchType::StartsWith => lv.starts_with(&lk),
        // CaseInsensitive / Substring / Wildcard / Fuzzy / FuzzySubstring -> substring contains
        _ => lv.contains(&lk),
    }
}

/// Map a PackageMatchField to an entry field name.
fn field_to_key(field: PackageMatchField) -> Option<&'static str> {
    match field {
        PackageMatchField::PackageIdentifier => Some("id"),
        PackageMatchField::PackageName => Some("name"),
        PackageMatchField::Publisher => Some("publisher"),
        PackageMatchField::Moniker => Some("monikers"),
        PackageMatchField::Command => Some("commands"),
        PackageMatchField::Tag => Some("tags"),
        PackageMatchField::PackageFamilyName => Some("packageFamilyNames"),
        PackageMatchField::ProductCode => Some("productCodes"),
        PackageMatchField::UpgradeCode => Some("upgradeCodes"),
        _ => None,
    }
}

/// Whether an entry field matches (array fields use any-match).
fn matches_field(
    entry: &WinGetSearchEntry,
    field: &str,
    keyword: &str,
    match_type: MatchType,
) -> bool {
    match field {
        "id" => match_string(&entry.id, keyword, match_type),
        "name" => match_string(&entry.name, keyword, match_type),
        "publisher" => match_string(&entry.publisher, keyword, match_type),
        "monikers" => entry
            .monikers
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        "tags" => entry
            .tags
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        "commands" => entry
            .commands
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        "packageFamilyNames" => entry
            .package_family_names
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        "productCodes" => entry
            .product_codes
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        "upgradeCodes" => entry
            .upgrade_codes
            .iter()
            .any(|v| match_string(v, keyword, match_type)),
        _ => false,
    }
}

/// Apply inclusions (AND): the entry must match every inclusion.
fn matches_inclusions(entry: &WinGetSearchEntry, inclusions: &[PackageMatchFilter]) -> bool {
    inclusions.iter().all(|inc| {
        let Some(kw) = inc.request_match.key_word.as_deref() else {
            return true;
        };
        if kw.is_empty() {
            return true;
        }
        let mt = inc.request_match.match_type.unwrap_or_default();
        if inc.package_match_field == PackageMatchField::NormalizedPackageNameAndPublisher {
            return match_string(&entry.name, kw, mt) || match_string(&entry.publisher, kw, mt);
        }
        match field_to_key(inc.package_match_field) {
            Some(key) => matches_field(entry, key, kw, mt),
            None => true,
        }
    })
}

/// Apply filters (NOT): the entry must not match any filter.
fn matches_filters(entry: &WinGetSearchEntry, filters: &[PackageMatchFilter]) -> bool {
    !filters.iter().any(|f| {
        let Some(kw) = f.request_match.key_word.as_deref() else {
            return false;
        };
        if kw.is_empty() {
            return false;
        }
        let mt = f.request_match.match_type.unwrap_or_default();
        if f.package_match_field == PackageMatchField::NormalizedPackageNameAndPublisher {
            return match_string(&entry.name, kw, mt) || match_string(&entry.publisher, kw, mt);
        }
        match field_to_key(f.package_match_field) {
            Some(key) => matches_field(entry, key, kw, mt),
            None => false,
        }
    })
}

/// Flatten to a searchable string list (field order drives scoring — mirrors scoreEntryKeyword).
fn entry_searchable_strings(entry: &WinGetSearchEntry) -> Vec<&str> {
    let mut v: Vec<&str> = Vec::with_capacity(
        3 + entry.monikers.len()
            + entry.tags.len()
            + entry.commands.len()
            + entry.package_family_names.len()
            + entry.product_codes.len()
            + entry.upgrade_codes.len(),
    );
    v.push(entry.id.as_str());
    v.push(entry.name.as_str());
    v.push(entry.publisher.as_str());
    v.extend(entry.monikers.iter().map(String::as_str));
    v.extend(entry.tags.iter().map(String::as_str));
    v.extend(entry.commands.iter().map(String::as_str));
    v.extend(entry.package_family_names.iter().map(String::as_str));
    v.extend(entry.product_codes.iter().map(String::as_str));
    v.extend(entry.upgrade_codes.iter().map(String::as_str));
    v
}

/// Linear scoring for non-Fuzzy match types (mirrors search.ts scoreEntryKeyword).
fn score_entry_keyword(entry: &WinGetSearchEntry, keyword: &str, match_type: MatchType) -> f64 {
    let kw = keyword.to_lowercase();
    let fields = entry_searchable_strings(entry);
    let mut best = 0.0_f64;
    for (i, f) in fields.iter().enumerate() {
        let fl = f.to_lowercase();
        let fl_len = fl.len().max(1) as f64;
        let (matched, score) = match match_type {
            MatchType::Exact => {
                let m = fl == kw;
                (m, if m { 1000.0 - i as f64 } else { 0.0 })
            }
            MatchType::StartsWith => {
                let m = fl.starts_with(&kw);
                (
                    m,
                    if m {
                        1000.0 - i as f64 + (kw.len() as f64 / fl_len) * 100.0
                    } else {
                        0.0
                    },
                )
            }
            // CaseInsensitive / Substring / Wildcard / FuzzySubstring / Fuzzy(fallback)
            _ => {
                let m = fl.contains(&kw);
                (
                    m,
                    if m {
                        1000.0 - i as f64 + (kw.len() as f64 / fl_len) * 100.0
                    } else {
                        0.0
                    },
                )
            }
        };
        if score > best {
            best = score;
        }
        if matched && matches!(match_type, MatchType::Exact | MatchType::CaseInsensitive) {
            return best;
        }
    }
    best
}

/// Weighted FuzzySearch score, replicating @nlptools/distance FuzzySearch.
///
/// extractKeyValue() returns the value only when `typeof value === "string"`;
/// array fields (monikers/tags/commands/packageFamilyNames/productCodes/upgradeCodes)
/// are therefore treated as "" and contribute 0, but their weights still count
/// in the normalization denominator. Only id/name/publisher actually participate.
fn fuzzy_score(entry: &WinGetSearchEntry, query_lower: &str) -> f64 {
    let mut score = 0.0_f64;
    score +=
        (WEIGHT_ID / TOTAL_WEIGHT) * normalized_levenshtein(query_lower, &entry.id.to_lowercase());
    score += (WEIGHT_NAME / TOTAL_WEIGHT)
        * normalized_levenshtein(query_lower, &entry.name.to_lowercase());
    score += (WEIGHT_PUBLISHER / TOTAL_WEIGHT)
        * normalized_levenshtein(query_lower, &entry.publisher.to_lowercase());
    // Array fields contribute 0 (extractKeyValue returns "" for non-strings).
    score
}

/// Main search entry point (mirrors search.ts searchPackages).
#[allow(clippy::too_many_arguments)]
pub fn search_packages(
    index: &[WinGetSearchEntry],
    keyword: Option<&str>,
    match_type: MatchType,
    maximum_results: Option<usize>,
    continuation_token: Option<&str>,
    inclusions: Option<&[PackageMatchFilter]>,
    filters: Option<&[PackageMatchFilter]>,
) -> SearchResult {
    let offset = decode_continuation_token(continuation_token);
    let inclusions = inclusions.unwrap_or(&[]);
    let filters = filters.unwrap_or(&[]);

    let has_keyword = keyword.is_some_and(|k| !k.is_empty());

    let mut candidates: Vec<&WinGetSearchEntry> = if !has_keyword
        && inclusions.is_empty()
        && filters.is_empty()
    {
        index.iter().collect()
    } else {
        let mut cands: Vec<&WinGetSearchEntry> = if has_keyword {
            let kw = keyword.unwrap();
            let is_fuzzy = matches!(match_type, MatchType::Fuzzy | MatchType::FuzzySubstring);
            if is_fuzzy {
                let threshold = if matches!(match_type, MatchType::Fuzzy) {
                    0.15
                } else {
                    0.10
                };
                let ql = kw.to_lowercase();
                let mut scored: Vec<(f64, &WinGetSearchEntry)> = index
                    .iter()
                    .map(|e| (fuzzy_score(e, &ql), e))
                    .filter(|(s, _)| *s >= threshold)
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored.into_iter().map(|(_, e)| e).collect()
            } else {
                let mut scored: Vec<(f64, &WinGetSearchEntry)> = index
                    .iter()
                    .filter_map(|e| {
                        let s = score_entry_keyword(e, kw, match_type);
                        if s > 0.0 { Some((s, e)) } else { None }
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored.into_iter().map(|(_, e)| e).collect()
            }
        } else {
            index.iter().collect()
        };

        if !inclusions.is_empty() {
            cands.retain(|e| matches_inclusions(e, inclusions));
        }
        if !filters.is_empty() {
            cands.retain(|e| matches_filters(e, filters));
        }
        cands
    };

    let total = candidates.len();
    let results: Vec<ManifestSearchResult> = match maximum_results {
        Some(max) => candidates
            .drain(..)
            .skip(offset)
            .take(max)
            .map(entry_to_result)
            .collect(),
        None => candidates
            .drain(..)
            .skip(offset)
            .map(entry_to_result)
            .collect(),
    };
    let has_more = total > offset + results.len();
    SearchResult {
        results,
        has_more,
        offset,
    }
}

fn entry_to_result(e: &WinGetSearchEntry) -> ManifestSearchResult {
    ManifestSearchResult {
        package_identifier: e.id.clone(),
        package_name: e.name.clone(),
        publisher: e.publisher.clone(),
        versions: e.versions.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winget::utils::token::encode_continuation_token;

    fn sample_index() -> Vec<WinGetSearchEntry> {
        vec![
            WinGetSearchEntry {
                id: "Microsoft.VisualStudioCode".to_string(),
                name: "Visual Studio Code".to_string(),
                publisher: "Microsoft".to_string(),
                monikers: vec!["vscode".to_string()],
                tags: vec!["editor".to_string()],
                commands: vec!["code".to_string()],
                versions: vec![ManifestVersion {
                    package_version: "1.95.0".to_string(),
                    channel: None,
                }],
                package_family_names: vec![],
                product_codes: vec![],
                upgrade_codes: vec![],
            },
            WinGetSearchEntry {
                id: "Git.Git".to_string(),
                name: "Git".to_string(),
                publisher: "Git".to_string(),
                monikers: vec![],
                tags: vec!["vcs".to_string()],
                commands: vec!["git".to_string()],
                versions: vec![ManifestVersion {
                    package_version: "2.40.0".to_string(),
                    channel: None,
                }],
                package_family_names: vec![],
                product_codes: vec![],
                upgrade_codes: vec![],
            },
        ]
    }

    #[test]
    fn fuzzy_matches_by_id_name_publisher() {
        // FuzzySearch only scores id/name/publisher (array fields are ignored).
        let idx = sample_index();
        let res = search_packages(&idx, Some("git"), MatchType::Fuzzy, None, None, None, None);
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].package_identifier, "Git.Git");
    }

    #[test]
    fn substring_case_insensitive() {
        let idx = sample_index();
        let res = search_packages(
            &idx,
            Some("visual"),
            MatchType::CaseInsensitive,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results.len(), 1);
    }

    #[test]
    fn pagination_has_more() {
        let idx = sample_index();
        let res = search_packages(
            &idx,
            None,
            MatchType::CaseInsensitive,
            Some(1),
            None,
            None,
            None,
        );
        assert_eq!(res.results.len(), 1);
        assert!(res.has_more);
    }

    #[test]
    fn continuation_token_roundtrip() {
        let idx = sample_index();
        let first = search_packages(
            &idx,
            None,
            MatchType::CaseInsensitive,
            Some(1),
            None,
            None,
            None,
        );
        let token = encode_continuation_token(first.offset + first.results.len());
        let second = search_packages(
            &idx,
            None,
            MatchType::CaseInsensitive,
            Some(1),
            Some(&token),
            None,
            None,
        );
        assert_eq!(second.results.len(), 1);
        assert_eq!(second.results[0].package_identifier, "Git.Git");
    }
}
