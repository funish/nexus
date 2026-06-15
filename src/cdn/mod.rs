pub mod routes;
pub mod utils;

/// CDN routes: `/cdn/{npm,jsr,gh,cdnjs,wp}/**` — behavior aligned with jsDelivr.
pub fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/cdn/npm/{*path}", get(routes::npm::handle_npm))
        .route("/cdn/jsr/{*path}", get(routes::jsr::handle_jsr))
        .route("/cdn/gh/{*path}", get(routes::gh::handle_gh))
        .route("/cdn/cdnjs/{*path}", get(routes::cdnjs::handle_cdnjs))
        .route("/cdn/wp/{*path}", get(routes::wp::handle_wp))
        .route(
            "/cdn/combine/{*paths}",
            get(routes::combine::handle_combine),
        )
}
