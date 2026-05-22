//! Force-directed graph view + JSON data feed.

use crate::{routes::vault::ApiError, AppState};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse},
    Json,
};
use ctxk_store::sqlite::ListFilters;
use serde::Serialize;

const GRAPH_HTML: &str = include_str!("../../templates/graph.html");
const D3_JS: &str = include_str!("../../assets/d3.v7.min.js");

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub knowledge_type: String,
    pub scope: String,
    pub status: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub rel: String,
}

#[derive(Debug, Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

pub async fn page() -> Html<&'static str> {
    Html(GRAPH_HTML)
}

pub async fn d3_script() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    (StatusCode::OK, headers, D3_JS)
}

pub async fn data(State(state): State<AppState>) -> Result<Json<GraphData>, ApiError> {
    // Pull every active item; cap generous default so the browser doesn't choke.
    let filters = ListFilters {
        scope: None,
        knowledge_type: None,
        domain: None,
        tag: None,
        q: None,
        include_status: vec!["active".to_string()],
        limit: Some(5000),
        offset: None,
    };
    let items = state
        .vault
        .store
        .list_items(&filters)
        .map_err(ApiError::internal)?;

    let nodes: Vec<GraphNode> = items
        .iter()
        .map(|it| GraphNode {
            id: it.id.clone(),
            title: if it.title.is_empty() {
                it.id.clone()
            } else {
                it.title.clone()
            },
            knowledge_type: it.knowledge_type.as_str().to_string(),
            scope: it.scope.as_str().to_string(),
            status: it.status.as_str().to_string(),
            snippet: it.body_text.chars().take(160).collect(),
        })
        .collect();

    let edges: Vec<GraphEdge> = items
        .iter()
        .flat_map(|it| {
            it.relations.iter().map(|r| GraphEdge {
                source: it.id.clone(),
                target: r.target.clone(),
                rel: r.rel.clone(),
            })
        })
        .collect();

    Ok(Json(GraphData { nodes, edges }))
}
