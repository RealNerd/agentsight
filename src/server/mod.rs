pub mod cache;
pub mod handlers;
pub mod sse;
pub mod state;
pub mod watcher;

use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;

use self::state::AppState;

#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

/// Build the full axum router with API routes and embedded static assets.
///
/// No CORS layer — the SPA is served from the same origin as the API,
/// and the server binds to 127.0.0.1 only (no cross-origin access needed).
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/sessions", get(handlers::list_sessions))
        .route("/sessions/{id}", get(handlers::get_session))
        .route(
            "/sessions/by-slug/{slug}",
            get(handlers::get_session_by_slug),
        )
        .route("/summary", get(handlers::get_summary))
        .route("/config", get(handlers::get_config))
        .route("/projects", get(handlers::list_projects))
        .route("/timeline", get(handlers::get_timeline))
        .route("/diagnose", get(handlers::get_diagnose))
        .route("/watch/stream", get(sse::watch_stream))
        .route("/health", get(handlers::health));

    Router::new()
        .nest("/api/v1", api)
        .fallback(static_handler)
        .with_state(state)
}

/// Serve embedded static assets, falling back to index.html for SPA routing.
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try exact file match first
    if !path.is_empty() {
        if let Some(file) = StaticAssets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref())
                        .unwrap_or(HeaderValue::from_static("application/octet-stream")),
                )],
                file.data.to_vec(),
            )
                .into_response();
        }
    }

    // SPA fallback: serve index.html for all non-file routes
    match StaticAssets::get("index.html") {
        Some(file) => Html(String::from_utf8_lossy(&file.data).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

/// Start the server on the given port.
pub async fn start_server(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    eprintln!("Dashboard running at http://127.0.0.1:{}", port);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c");
    eprintln!("\nShutting down dashboard server...");
}
