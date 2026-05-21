//! ContextKernel HTTP server.
//!
//! All endpoints are JSON in / JSON out. Loopback by default; no auth in
//! MVP (the spec calls for local-first operation with explicit per-call
//! AI provider approval — bearer-token auth lands in phase 3).

use axum::{
    routing::{get, patch, post},
    Router,
};
use ctxk_store::Vault;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub vault: Arc<Vault>,
}

pub fn build_router(vault: Arc<Vault>) -> Router {
    let state = AppState { vault };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health + stats
        .route("/health", get(routes::vault::health))
        .route("/vault/stats", get(routes::vault::stats))
        .route("/vault/reindex", post(routes::vault::reindex))
        // Context query — the flagship
        .route("/context/query", post(routes::context::query))
        // Knowledge read/list
        .route("/knowledge", get(routes::knowledge::list))
        .route("/knowledge/:id", get(routes::knowledge::get_one))
        // Propose / review
        .route("/knowledge/propose", post(routes::review::propose_new))
        .route(
            "/knowledge/:id/propose-update",
            patch(routes::review::propose_update),
        )
        .route("/review/queue", get(routes::review::queue_list))
        .with_state(state)
        .layer(cors)
}

pub async fn serve(vault: Arc<Vault>, bind: &str) -> anyhow::Result<()> {
    let app = build_router(vault);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    tracing::info!("ctxk-server listening on http://{}", addr);
    println!("Listening on http://{}", addr);
    println!("  GET  /health");
    println!("  GET  /vault/stats");
    println!("  POST /vault/reindex");
    println!("  POST /context/query        body: {{ task, scope, ... }}");
    println!("  GET  /knowledge?scope=&type=&domain=&tag=&q=");
    println!("  GET  /knowledge/{{id}}");
    println!("  POST /knowledge/propose");
    println!("  PATCH /knowledge/{{id}}/propose-update");
    println!("  GET  /review/queue?status=pending");
    axum::serve(listener, app).await?;
    Ok(())
}
