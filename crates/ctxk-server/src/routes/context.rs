use crate::{routes::vault::ApiError, AppState};
use axum::{extract::State, Json};
use ctxk_retrieval::{assemble, execute, ContextBundle, Query};

pub async fn query(
    State(state): State<AppState>,
    Json(req): Json<Query>,
) -> Result<Json<ContextBundle>, ApiError> {
    let scored = execute(&state.vault.store, &req).map_err(ApiError::internal)?;
    let bundle = assemble(&state.vault.store, &req, scored).map_err(ApiError::internal)?;
    Ok(Json(bundle))
}
