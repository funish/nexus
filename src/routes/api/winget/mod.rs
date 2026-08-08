//! WinGet REST API routes: `/api/winget/**` — aligned with the WinGet RESTSource spec.

pub mod catalog;
pub mod manifests;

pub fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route(
            "/api/winget/manifestSearch",
            get(catalog::handle_manifest_search_get)
                .post(catalog::handle_manifest_search_post),
        )
        .route(
            "/api/winget/packages",
            get(catalog::handle_packages),
        )
        .route(
            "/api/winget/information",
            get(catalog::handle_information),
        )
        .route(
            "/api/winget/packages/{id}",
            get(catalog::handle_package),
        )
        .route(
            "/api/winget/packages/{id}/versions",
            get(manifests::handle_versions),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/installers",
            get(manifests::handle_installers),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/installers/{installer}",
            get(manifests::handle_installer),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/locales",
            get(manifests::handle_locales),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/locales/{locale}",
            get(manifests::handle_locale),
        )
        .route(
            "/api/winget/packageManifests/{id}",
            get(manifests::handle_package_manifest),
        )
}
