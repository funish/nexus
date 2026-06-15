mod cdn;
mod config;
mod error;
mod http;
mod storage;
mod winget;

use axum::Router;
use axum::http::{HeaderName, HeaderValue};
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing_subscriber::EnvFilter;

/// Shared application state passed to all handlers.
pub type AppState = (storage::SharedStorage, winget::utils::db::SharedDb);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nexus=info".parse().unwrap()))
        .init();

    let config = config::Config::from_env();
    let storage = storage::create_storage(&config).await;
    let winget_db = winget::utils::db::create_shared_db();

    // Permissive CORS for a public CDN: any origin, method, and request header.
    // Safelisted response headers (cache-control, content-length, content-type, ...)
    // are browser-readable by default; expose_headers adds the custom ones
    // cross-origin JS needs — etag for conditional requests and x-resolved-version
    // so callers learn the exact version a range/tag resolved to.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers([
            HeaderName::from_static("etag"),
            HeaderName::from_static("x-resolved-version"),
            HeaderName::from_static("last-modified"),
        ]);

    // Gzip JS/CSS/JSON responses; tower-http skips already-compressed types
    // (images, fonts, archives) and honors the client's Accept-Encoding.
    let compression = CompressionLayer::new().gzip(true);

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
        // Order matters: cors is outermost (intercepts OPTIONS preflight and tags
        // every response with CORS headers), compression is inner (compresses
        // handler bodies before cors adds its headers). nosniff is innermost — a
        // blanket response header applied to every file served.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(compression)
        .layer(cors)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("Nexus CDN listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
