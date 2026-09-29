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

use super::rest::{
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
    match match_type {
        MatchType::Exact => value == keyword,
        MatchType::CaseInsensitive => value.to_lowercase() == keyword.to_lowercase(),
        MatchType::StartsWith => value.to_lowercase().starts_with(&keyword.to_lowercase()),
        MatchType::Substring => {
            let lv = value.to_lowercase();
            let lk = keyword.to_lowercase();
            lv.contains(&lk)
        }
        // Keep the legacy wildcard/fuzzy fallbacks usable for advanced requests;
        // the reference REST source does not expose these operators.
        MatchType::Wildcard | MatchType::Fuzzy | MatchType::FuzzySubstring => {
            let lv = value.to_lowercase();
            let lk = keyword.to_lowercase();
            lv.contains(&lk)
        }
    }
}

/// Normalize the loosely spelled name used by NormalizedPackageNameAndPublisher.
fn normalized_name(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '+'))
        .collect()
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

/// Whether one inclusion or filter matches its declared field.
fn filter_matches(entry: &WinGetSearchEntry, filter: &PackageMatchFilter) -> bool {
    let Some(kw) = filter.request_match.key_word.as_deref() else {
        return true;
    };
    if kw.is_empty() {
        return true;
    }
    let mt = filter.request_match.match_type.unwrap_or_default();

    if filter.package_match_field == PackageMatchField::NormalizedPackageNameAndPublisher {
        return match_string(&normalized_name(&entry.name), &normalized_name(kw), mt);
    }

    // Fields reported as unsupported are ignored, matching the reference store's
    // behavior of omitting unsupported predicates rather than rejecting matches.
    field_to_key(filter.package_match_field).is_none_or(|key| matches_field(entry, key, kw, mt))
}

/// Apply inclusions (OR): one matching inclusion is sufficient.
fn matches_inclusions(entry: &WinGetSearchEntry, inclusions: &[PackageMatchFilter]) -> bool {
    inclusions.is_empty() || inclusions.iter().any(|inc| filter_matches(entry, inc))
}

/// Apply filters (AND): every specified filter must match.
fn matches_filters(entry: &WinGetSearchEntry, filters: &[PackageMatchFilter]) -> bool {
    filters.iter().all(|filter| filter_matches(entry, filter))
}

/// Score one value by how strongly it matches the keyword.
fn match_specificity(value: &str, keyword: &str, match_type: MatchType) -> Option<u64> {
    let matched = match match_type {
        MatchType::Exact => value == keyword,
        MatchType::CaseInsensitive => value.to_lowercase() == keyword.to_lowercase(),
        MatchType::StartsWith => value.to_lowercase().starts_with(&keyword.to_lowercase()),
        MatchType::Substring => {
            let lv = value.to_lowercase();
            let lk = keyword.to_lowercase();
            lv.contains(&lk)
        }
        MatchType::Wildcard | MatchType::Fuzzy | MatchType::FuzzySubstring => {
            let lv = value.to_lowercase();
            let lk = keyword.to_lowercase();
            lv.contains(&lk)
        }
    };
    if !matched {
        return None;
    }

    let lv = value.to_lowercase();
    let lk = keyword.to_lowercase();
    if lv == lk {
        return Some(5_000);
    }
    if lv.starts_with(&lk) {
        return Some(3_000);
    }

    let token_equal = value
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| token.to_lowercase() == lk);
    if token_equal {
        return Some(2_000);
    }
    let token_starts = value
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| token.to_lowercase().starts_with(&lk));
    Some(if token_starts { 1_000 } else { 0 })
}

fn score_identity_group(value: &str, keyword: &str, match_type: MatchType, base: u64) -> u64 {
    match_specificity(value, keyword, match_type).map_or(0, |mut specificity| {
        let parts = value.split(|c: char| !c.is_alphanumeric()).count();
        specificity += 500 - parts.saturating_sub(1).min(5) as u64 * 100;
        base + specificity
    })
}

fn score_field_group(value: &str, keyword: &str, match_type: MatchType, base: u64) -> u64 {
    match_specificity(value, keyword, match_type).map_or(0, |specificity| base + specificity)
}

fn score_field_group_any(
    values: &[String],
    keyword: &str,
    match_type: MatchType,
    base: u64,
) -> u64 {
    values
        .iter()
        .map(|value| score_field_group(value, keyword, match_type, base))
        .max()
        .unwrap_or_default()
}

/// Rank identity, name, publisher, then the best auxiliary field. Tuples prevent
/// auxiliary metadata from overriding a stronger PackageIdentifier/PackageName.
type KeywordScore = (u64, u64, u64, u64);

fn score_entry_keyword(
    entry: &WinGetSearchEntry,
    keyword: &str,
    match_type: MatchType,
) -> KeywordScore {
    let auxiliary = [
        score_field_group_any(&entry.monikers, keyword, match_type, 50_000),
        score_field_group_any(&entry.tags, keyword, match_type, 10_000),
        score_field_group_any(&entry.commands, keyword, match_type, 5_000),
        score_field_group_any(&entry.package_family_names, keyword, match_type, 1_000),
        score_field_group_any(&entry.product_codes, keyword, match_type, 500),
        score_field_group_any(&entry.upgrade_codes, keyword, match_type, 100),
    ]
    .into_iter()
    .max()
    .unwrap_or_default();

    (
        score_identity_group(&entry.id, keyword, match_type, 1_000_000),
        score_field_group(&entry.name, keyword, match_type, 500_000),
        score_field_group(&entry.publisher, keyword, match_type, 100_000),
        auxiliary,
    )
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

    let mut candidates: Vec<&WinGetSearchEntry> =
        if !has_keyword && inclusions.is_empty() && filters.is_empty() {
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
                    scored.sort_by(|a, b| {
                        b.0.partial_cmp(&a.0)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| a.1.id.cmp(&b.1.id))
                    });
                    scored.into_iter().map(|(_, e)| e).collect()
                } else {
                    let mut scored: Vec<(KeywordScore, &WinGetSearchEntry)> = index
                        .iter()
                        .filter_map(|e| {
                            let s = score_entry_keyword(e, kw, match_type);
                            (s != (0, 0, 0, 0)).then_some((s, e))
                        })
                        .collect();
                    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
                    scored.into_iter().map(|(_, e)| e).collect()
                }
            } else {
                let mut cands = index.iter().collect::<Vec<_>>();
                cands.sort_by(|a, b| a.id.cmp(&b.id));
                cands
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
    use crate::winget::token::encode_continuation_token;

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
    fn substring_matches_case_insensitively() {
        let idx = sample_index();
        let res = search_packages(
            &idx,
            Some("visual"),
            MatchType::Substring,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results.len(), 1);
    }

    #[test]
    fn case_insensitive_requires_the_whole_field() {
        let idx = sample_index();
        assert!(
            search_packages(
                &idx,
                Some("Visual"),
                MatchType::CaseInsensitive,
                None,
                None,
                None,
                None
            )
            .results
            .is_empty()
        );
        let res = search_packages(
            &idx,
            Some("visual studio code"),
            MatchType::CaseInsensitive,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results.len(), 1);
    }

    fn match_filter(
        field: crate::winget::rest::PackageMatchField,
        keyword: &str,
        match_type: MatchType,
    ) -> crate::winget::rest::PackageMatchFilter {
        crate::winget::rest::PackageMatchFilter {
            package_match_field: field,
            request_match: crate::winget::rest::SearchRequestMatch {
                key_word: Some(keyword.to_string()),
                match_type: Some(match_type),
                package_match_field: None,
            },
        }
    }

    #[test]
    fn inclusions_are_or_filters_are_and() {
        let idx = sample_index();
        let inclusions = [
            match_filter(
                crate::winget::rest::PackageMatchField::PackageIdentifier,
                "Git.Git",
                MatchType::Exact,
            ),
            match_filter(
                crate::winget::rest::PackageMatchField::PackageName,
                "Visual Studio Code",
                MatchType::Exact,
            ),
        ];
        let res = search_packages(
            &idx,
            None,
            MatchType::Substring,
            None,
            None,
            Some(&inclusions),
            None,
        );
        assert_eq!(res.results.len(), 2);

        let filters = [
            match_filter(
                crate::winget::rest::PackageMatchField::Publisher,
                "Microsoft",
                MatchType::Exact,
            ),
            match_filter(
                crate::winget::rest::PackageMatchField::Moniker,
                "vscode",
                MatchType::Exact,
            ),
        ];
        let res = search_packages(
            &idx,
            None,
            MatchType::Substring,
            None,
            None,
            None,
            Some(&filters),
        );
        assert_eq!(res.results.len(), 1);
        assert_eq!(
            res.results[0].package_identifier,
            "Microsoft.VisualStudioCode"
        );
    }

    #[test]
    fn starts_with_ignores_case() {
        let idx = sample_index();
        let res = search_packages(
            &idx,
            Some("VIS"),
            MatchType::StartsWith,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results.len(), 1);
        assert_eq!(
            res.results[0].package_identifier,
            "Microsoft.VisualStudioCode"
        );
    }

    #[test]
    fn shorter_identifiers_rank_above_longer_variants() {
        let idx = vec![
            WinGetSearchEntry {
                id: "Google.Chrome.Canary".to_string(),
                name: "Google Chrome Canary".to_string(),
                publisher: "Google LLC".to_string(),
                monikers: vec![],
                tags: vec![],
                commands: vec![],
                versions: Vec::new(),
                package_family_names: vec![],
                product_codes: vec![],
                upgrade_codes: vec![],
            },
            WinGetSearchEntry {
                id: "Google.Chrome".to_string(),
                name: "Google Chrome".to_string(),
                publisher: "Google LLC".to_string(),
                monikers: vec![],
                tags: vec![],
                commands: vec![],
                versions: Vec::new(),
                package_family_names: vec![],
                product_codes: vec![],
                upgrade_codes: vec![],
            },
        ];
        let res = search_packages(
            &idx,
            Some("chrome"),
            MatchType::Substring,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results[0].package_identifier, "Google.Chrome");
    }
    #[test]
    fn primary_fields_rank_above_commands() {
        let mut idx = sample_index();
        idx.push(WinGetSearchEntry {
            id: "Other.Tool".to_string(),
            name: "Other Tool".to_string(),
            publisher: "Other".to_string(),
            monikers: vec![],
            tags: vec![],
            commands: vec!["git".to_string()],
            versions: Vec::new(),
            package_family_names: vec![],
            product_codes: vec![],
            upgrade_codes: vec![],
        });
        let res = search_packages(
            &idx,
            Some("git"),
            MatchType::Substring,
            None,
            None,
            None,
            None,
        );
        assert_eq!(res.results[0].package_identifier, "Git.Git");
    }

    #[test]
    fn pagination_has_more() {
        let idx = sample_index();
        let res = search_packages(&idx, None, MatchType::Substring, Some(1), None, None, None);
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
            MatchType::Substring,
            Some(1),
            Some(&token),
            None,
            None,
        );
        assert_eq!(second.results.len(), 1);
        assert_eq!(second.results[0].package_identifier, "Git.Git");
    }
}
