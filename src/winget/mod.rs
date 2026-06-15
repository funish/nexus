pub mod routes;
pub mod utils;

/// WinGet REST API routes: `/api/winget/**` — aligned with the WinGet RESTSource spec.
pub fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route(
            "/api/winget/manifestSearch",
            get(routes::catalog::handle_manifest_search_get)
                .post(routes::catalog::handle_manifest_search_post),
        )
        .route(
            "/api/winget/packages",
            get(routes::catalog::handle_packages),
        )
        .route(
            "/api/winget/information",
            get(routes::catalog::handle_information),
        )
        .route(
            "/api/winget/packages/{id}",
            get(routes::catalog::handle_package),
        )
        .route(
            "/api/winget/packages/{id}/versions",
            get(routes::manifests::handle_versions),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/installers",
            get(routes::manifests::handle_installers),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/installers/{installer}",
            get(routes::manifests::handle_installer),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/locales",
            get(routes::manifests::handle_locales),
        )
        .route(
            "/api/winget/packages/{id}/versions/{version}/locales/{locale}",
            get(routes::manifests::handle_locale),
        )
        .route(
            "/api/winget/packageManifests/{id}",
            get(routes::manifests::handle_package_manifest),
        )
}
