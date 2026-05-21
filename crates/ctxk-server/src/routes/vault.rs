use crate::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

pub async fn health() -> &'static str {
    "ok"
}

pub async fn stats(State(state): State<AppState>) -> Result<Json<ctxk_store::sqlite::VaultStats>, ApiError> {
    let stats = state.vault.store.stats().map_err(ApiError::internal)?;
    Ok(Json(stats))
}

#[derive(Debug, Deserialize, Default)]
pub struct ReindexBody {
    /// Optional relative file path. Reindex everything if omitted.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReindexResponse {
    pub files_scanned: usize,
    pub items_indexed: usize,
    pub files_skipped: usize,
    pub errors: Vec<String>,
}

pub async fn reindex(
    State(state): State<AppState>,
    body: Option<Json<ReindexBody>>,
) -> Result<Json<ReindexResponse>, ApiError> {
    let _ = body; // path-scoped reindex is a session-2 feature; full only for now
    let report = state.vault.reindex_all().map_err(ApiError::internal)?;
    Ok(Json(ReindexResponse {
        files_scanned: report.files_scanned,
        items_indexed: report.items_indexed,
        files_skipped: report.files_skipped,
        errors: report
            .errors
            .into_iter()
            .map(|(p, e)| format!("{}: {}", p.display(), e))
            .collect(),
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Error wrapper used by all routes
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "error": {
                "status": self.status.as_u16(),
                "message": self.message,
            }
        });
        (self.status, Json(body)).into_response()
    }
}
