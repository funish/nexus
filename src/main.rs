mod cdn;
mod config;
mod error;
mod routes;
mod storage;
mod utils;
mod winget;

use axum::Router;
use axum::http::{HeaderName, HeaderValue};
use axum::routing::get;
use mimalloc::MiMalloc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL_ALLOCATOR: MiMalloc = MiMalloc;

/// Shared application state passed to all handlers.
pub type AppState = (storage::SharedStorage, winget::db::SharedDb);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("nexus=info".parse().unwrap()))
        .init();

    let config = config::Config::from_env();
    config::set_winget_runtime(&config);
    let storage = storage::create_storage(&config).await;
    let winget_db = winget::db::create_shared_db();

    // Permissive CORS for a public CDN: any origin, method, and request header,
    // and every response header exposed cross-origin (jsDelivr's
    // `access-control-expose-headers: *`) so browser JS can read etag and the
    // resolved-version headers for conditional requests and range aliases.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any);

    // Gzip JS/CSS/JSON responses; tower-http skips already-compressed types
    // (images, fonts, archives) and honors the client's Accept-Encoding.
    let compression = CompressionLayer::new().gzip(true);

    let state: AppState = (storage, winget_db);

    let app = Router::new()
        .merge(routes::cdn::router())
        .merge(routes::api::winget::router())
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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

/// Graceful shutdown: stops accepting new connections and waits for in-flight
/// requests to complete before returning. Handles both Ctrl+C (SIGINT) and
/// SIGTERM (Docker / systemd stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
