//! ContextKernel HTTP server.
//!
//! All endpoints are JSON in / JSON out. Loopback by default; no auth in
//! MVP (the spec calls for local-first operation with explicit per-call
//! AI provider approval — bearer-token auth lands in phase 3).

use axum::{
    routing::{get, patch, post},
    Router,
};
use ctxk_core::EmbedderProvider;
use ctxk_store::Vault;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub vault: Arc<Vault>,
    pub embedder: Option<Arc<dyn EmbedderProvider>>,
}

pub fn build_router(vault: Arc<Vault>, embedder: Option<Arc<dyn EmbedderProvider>>) -> Router {
    let state = AppState { vault, embedder };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health + stats
        .route("/health", get(routes::vault::health))
        .route("/vault/stats", get(routes::vault::stats))
        .route("/vault/reindex", post(routes::vault::reindex))
        .route("/vault/reembed", post(routes::vault::reembed))
        // Context query
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
        .route(
            "/review/queue/:queue_id/approve",
            post(routes::review::approve),
        )
        .route(
            "/review/queue/:queue_id/reject",
            post(routes::review::reject),
        )
        // Graph view
        .route("/graph", get(routes::graph::page))
        .route("/graph/data", get(routes::graph::data))
        .route("/d3.v7.min.js", get(routes::graph::d3_script))
        .with_state(state)
        .layer(cors)
}

pub async fn serve(
    vault: Arc<Vault>,
    embedder: Option<Arc<dyn EmbedderProvider>>,
    bind: &str,
) -> anyhow::Result<()> {
    let semantic = if embedder.is_some() { "ON" } else { "off" };
    let app = build_router(vault, embedder);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    tracing::info!("ctxk-server listening on http://{}", addr);
    println!("Listening on http://{} (semantic search: {})", addr, semantic);
    println!("  GET  /health");
    println!("  GET  /vault/stats");
    println!("  POST /vault/reindex");
    println!("  POST /vault/reembed                (force re-embed everything)");
    println!("  POST /context/query");
    println!("  GET  /knowledge?scope=&type=&domain=&tag=&q=");
    println!("  GET  /knowledge/{{id}}");
    println!("  POST /knowledge/propose");
    println!("  PATCH /knowledge/{{id}}/propose-update");
    println!("  GET  /review/queue?status=pending");
    println!("  POST /review/queue/{{id}}/approve  body: {{ target_file?: \"…\" }}");
    println!("  POST /review/queue/{{id}}/reject");
    println!("  GET  /graph                        force-graph HTML page");
    println!("  GET  /graph/data                   nodes + edges JSON");
    axum::serve(listener, app).await?;
    Ok(())
}
