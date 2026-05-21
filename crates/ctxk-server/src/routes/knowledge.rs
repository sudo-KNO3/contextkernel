use crate::{routes::vault::ApiError, AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use ctxk_core::KnowledgeItem;
use ctxk_store::sqlite::ListFilters;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct ListParams {
    pub scope: Option<String>,
    #[serde(rename = "type")]
    pub knowledge_type: Option<String>,
    pub domain: Option<String>,
    pub tag: Option<String>,
    pub q: Option<String>,
    #[serde(default)]
    pub include_stale: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(p): Query<ListParams>,
) -> Result<Json<Vec<KnowledgeItem>>, ApiError> {
    let include_status = if p.include_stale {
        vec![
            "active".to_string(),
            "stale".to_string(),
            "superseded".to_string(),
        ]
    } else {
        vec!["active".to_string()]
    };
    let filters = ListFilters {
        scope: p.scope,
        knowledge_type: p.knowledge_type,
        domain: p.domain,
        tag: p.tag,
        q: p.q,
        include_status,
        limit: p.limit.or(Some(100)),
        offset: p.offset,
    };
    let items = state
        .vault
        .store
        .list_items(&filters)
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<KnowledgeItem>, ApiError> {
    let item = state
        .vault
        .store
        .get_item(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("knowledge item not found: {id}")))?;
    Ok(Json(item))
}
