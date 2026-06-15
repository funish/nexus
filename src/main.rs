mod cdn;
mod config;
mod error;
mod storage;
mod winget;

use axum::Router;
use axum::routing::get;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

/// Shared application state passed to all handlers.
pub type AppState = (storage::SharedStorage, winget::utils::db::SharedDb);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nexus=info".parse().unwrap()))
        .init();

    let config = config::Config::from_env();
    let storage = storage::create_storage(&config);
    let winget_db = winget::utils::db::create_shared_db();

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let state: AppState = (storage, winget_db);

    let app = Router::new()
        .merge(cdn::router())
        .merge(winget::router())
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("../index.html")) }),
        )
        .route(
            "/favicon.ico",
            get(|| async {
                (
                    axum::http::StatusCode::OK,
                    [("content-type", "image/x-icon")],
                    &include_bytes!("../public/favicon.ico")[..],
                )
            }),
        )
        .layer(cors)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("Nexus CDN listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
