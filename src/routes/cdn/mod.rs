//! CDN routes: `/cdn/{npm,jsr,gh,cdnjs,wp}/**` — behavior aligned with jsDelivr.

pub mod cdnjs;
pub mod combine;
pub mod gh;
pub mod jsr;
pub mod npm;
pub mod wp;

pub fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/cdn/npm/{*path}", get(npm::handle_npm))
        .route("/cdn/jsr/{*path}", get(jsr::handle_jsr))
        .route("/cdn/gh/{*path}", get(gh::handle_gh))
        .route("/cdn/cdnjs/{*path}", get(cdnjs::handle_cdnjs))
        .route("/cdn/wp/{*path}", get(wp::handle_wp))
        .route(
            "/cdn/combine/{*paths}",
            get(combine::handle_combine),
        )
}
