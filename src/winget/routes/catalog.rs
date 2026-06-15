//! WinGet REST Source HTTP routes.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::storage::SharedStorage;
use crate::winget::utils::db::{SharedDb, get_index_db, get_search_index};
use crate::winget::utils::queries::package_exists;
use crate::winget::utils::response::{
    AuthenticationInfo, InformationData, InformationResponse, ManifestSearchRequest,
    ManifestSearchResponse, MatchType, PackageIdentifierItem, PackageMatchFilter,
    PackageSingleResponse, PackagesResponse, json_ok, winget_error,
};
use crate::winget::utils::search::{SearchResult, search_packages};
use crate::winget::utils::token::{decode_continuation_token, encode_continuation_token};

const WINGET_PACKAGES_PAGE_SIZE: usize = 100;

type AppState = (SharedStorage, SharedDb);

/// GET /api/winget/manifestSearch query params (compatibility mode).
#[derive(Deserialize)]
pub struct ManifestSearchParams {
    pub query: Option<String>,
    #[serde(rename = "matchType")]
    pub match_type: Option<MatchType>,
    #[serde(rename = "maximumResults")]
    pub maximum_results: Option<usize>,
}

/// GET /api/winget/manifestSearch
pub async fn handle_manifest_search_get(
    State((storage, db)): State<AppState>,
    Query(params): Query<ManifestSearchParams>,
    headers: HeaderMap,
) -> Response {
    let match_type = params.match_type.unwrap_or_default();
    let token = header_continuation_token(&headers);
    run_and_build(
        &db,
        &storage,
        params.query,
        match_type,
        params.maximum_results,
        token,
        None,
        None,
    )
    .await
}

/// POST /api/winget/manifestSearch
pub async fn handle_manifest_search_post(
    State((storage, db)): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ManifestSearchRequest>,
) -> Response {
    let keyword = req.query.as_ref().and_then(|q| q.key_word.clone());
    let match_type = req
        .query
        .as_ref()
        .and_then(|q| q.match_type)
        .unwrap_or_default();
    let token = header_continuation_token(&headers);
    run_and_build(
        &db,
        &storage,
        keyword,
        match_type,
        req.maximum_results,
        token,
        req.inclusions,
        req.filters,
    )
    .await
}

/// Shared search execution + response builder for GET/POST.
#[allow(clippy::too_many_arguments)]
async fn run_and_build(
    db: &SharedDb,
    storage: &SharedStorage,
    keyword: Option<String>,
    match_type: MatchType,
    maximum_results: Option<usize>,
    continuation_token: Option<String>,
    inclusions: Option<Vec<PackageMatchFilter>>,
    filters: Option<Vec<PackageMatchFilter>>,
) -> Response {
    let index = match get_search_index(db, storage).await {
        Ok(idx) => idx,
        Err(e) => return winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let SearchResult {
        results,
        has_more,
        offset,
    } = search_packages(
        &index,
        keyword.as_deref(),
        match_type,
        maximum_results,
        continuation_token.as_deref(),
        inclusions.as_deref(),
        filters.as_deref(),
    );

    if results.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let continuation_token = if has_more {
        Some(encode_continuation_token(offset + results.len()))
    } else {
        None
    };
    let body = ManifestSearchResponse {
        data: results,
        continuation_token,
        required_package_match_fields: vec!["PackageIdentifier".to_string()],
        unsupported_package_match_fields: vec![
            "Market".to_string(),
            "HasInstallerType".to_string(),
        ],
    };
    (
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("cache-control", "public, max-age=300"),
        ],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// GET /api/winget/packages query params.
#[derive(Deserialize)]
pub struct PackagesParams {
    #[serde(rename = "ContinuationToken")]
    pub continuation_token: Option<String>,
}

/// GET /api/winget/packages — list all package ids (paginated).
pub async fn handle_packages(
    State((storage, db)): State<AppState>,
    Query(params): Query<PackagesParams>,
) -> Response {
    let offset = decode_continuation_token(params.continuation_token.as_deref());
    let result = async {
        // Derive the id list from the cached search index instead of a fresh SQL
        // scan: this reuses the persisted index (winget/index.json) and avoids
        // triggering a full build on the packages endpoint.
        let index = get_search_index(&db, &storage).await?;
        let mut ids: Vec<String> = index.iter().map(|e| e.id.clone()).collect();
        ids.sort();
        anyhow::Ok(ids)
    }
    .await;

    match result {
        Ok(ids) => {
            let total = ids.len();
            let page: Vec<String> = ids
                .into_iter()
                .skip(offset)
                .take(WINGET_PACKAGES_PAGE_SIZE)
                .collect();
            let has_more = total > offset + page.len();
            let continuation_token = if has_more {
                Some(encode_continuation_token(offset + page.len()))
            } else {
                None
            };
            let body = PackagesResponse {
                data: page
                    .into_iter()
                    .map(|id| PackageIdentifierItem {
                        package_identifier: id,
                    })
                    .collect(),
                continuation_token,
            };
            json_ok(&body)
        }
        Err(e) => winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// GET /api/winget/information — static server information.
pub async fn handle_information() -> Response {
    let body = InformationResponse {
        data: InformationData {
            source_identifier: "Funish.Nexus".to_string(),
            server_supported_versions: vec!["1.4.0".to_string(), "1.9.0".to_string()],
            required_package_match_fields: vec!["PackageIdentifier".to_string()],
            unsupported_package_match_fields: vec![
                "Market".to_string(),
                "HasInstallerType".to_string(),
            ],
            unsupported_query_parameters: vec!["FetchAllManifests".to_string()],
            required_query_parameters: vec![],
            authentication: AuthenticationInfo {
                authentication_type: "none".to_string(),
            },
        },
    };
    json_ok(&body)
}

/// GET /api/winget/packages/:id — package existence.
pub async fn handle_package(
    State((storage, db)): State<AppState>,
    Path(package_id): Path<String>,
) -> Response {
    let result = async {
        let conn = get_index_db(&db, &storage).await?;
        let exists = package_exists(&conn, &package_id)?;
        anyhow::Ok(exists)
    }
    .await;
    match result {
        Ok(true) => json_ok(&PackageSingleResponse {
            data: PackageIdentifierItem {
                package_identifier: package_id,
            },
        }),
        Ok(false) => winget_error(
            StatusCode::NOT_FOUND,
            &format!("Package '{package_id}' not found"),
        ),
        Err(e) => winget_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Read the continuation token from the ContinuationToken header.
fn header_continuation_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("continuationtoken")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
