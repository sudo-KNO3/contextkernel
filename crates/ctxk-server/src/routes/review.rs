use crate::{routes::vault::ApiError, AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use ctxk_store::sqlite::QueueEntry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ProposeNew {
    pub proposed_by: String,
    #[serde(default)]
    pub rationale: Option<String>,
    pub item: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ProposeResponse {
    pub queue_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

pub async fn propose_new(
    State(state): State<AppState>,
    Json(req): Json<ProposeNew>,
) -> Result<Json<ProposeResponse>, ApiError> {
    if req.proposed_by.trim().is_empty() {
        return Err(ApiError::bad_request("proposed_by is required"));
    }
    let payload_json = serde_json::to_string(&req.item)
        .map_err(|e| ApiError::bad_request(format!("invalid item payload: {e}")))?;
    let queue_id = state
        .vault
        .store
        .queue_propose("new", None, &req.proposed_by, &payload_json, req.rationale.as_deref())
        .map_err(ApiError::internal)?;
    Ok(Json(ProposeResponse {
        queue_id,
        status: "pending".to_string(),
        target_id: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ProposeUpdate {
    pub proposed_by: String,
    #[serde(default)]
    pub rationale: Option<String>,
    pub patch: serde_json::Value,
}

pub async fn propose_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ProposeUpdate>,
) -> Result<Json<ProposeResponse>, ApiError> {
    let exists = state
        .vault
        .store
        .get_item(&id)
        .map_err(ApiError::internal)?;
    if exists.is_none() {
        return Err(ApiError::not_found(format!("target item not found: {id}")));
    }
    let payload_json = serde_json::to_string(&req.patch)
        .map_err(|e| ApiError::bad_request(format!("invalid patch payload: {e}")))?;
    let queue_id = state
        .vault
        .store
        .queue_propose(
            "update",
            Some(&id),
            &req.proposed_by,
            &payload_json,
            req.rationale.as_deref(),
        )
        .map_err(ApiError::internal)?;
    Ok(Json(ProposeResponse {
        queue_id,
        status: "pending".to_string(),
        target_id: Some(id),
    }))
}

#[derive(Debug, Deserialize, Default)]
pub struct QueueListParams {
    #[serde(default)]
    pub status: Option<String>,
}

pub async fn queue_list(
    State(state): State<AppState>,
    Query(p): Query<QueueListParams>,
) -> Result<Json<Vec<QueueEntry>>, ApiError> {
    let entries = state
        .vault
        .store
        .queue_list(p.status.as_deref())
        .map_err(ApiError::internal)?;
    Ok(Json(entries))
}
