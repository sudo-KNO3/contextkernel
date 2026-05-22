use crate::{routes::vault::ApiError, AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use ctxk_core::{
    new_id, KnowledgeItem, KnowledgeType, Relation, Scope, SourceType, Stability, Status,
};
use ctxk_store::{html_emit, sqlite::QueueEntry};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

// ─── propose ────────────────────────────────────────────────────────────────

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

// ─── queue list ─────────────────────────────────────────────────────────────

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

// ─── approve / reject ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct ApproveBody {
    /// Optional override: vault-relative path (POSIX). Otherwise the
    /// target file is derived from the item's scope + knowledge_type.
    #[serde(default)]
    pub target_file: Option<String>,
    /// Optional scope_path for project-scoped items
    /// (e.g. "demo" → projects/demo/<type>s.html).
    #[serde(default)]
    pub scope_path: Option<String>,
    /// Optional reviewer-supplied edits to merge into the proposed item
    /// before materialisation. Skipped for MVP — payload is taken as-is.
    #[serde(default)]
    pub edits: Option<serde_json::Value>,
    #[serde(default)]
    pub reviewed_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApproveResponse {
    pub queue_id: String,
    pub item_id: String,
    pub status: String,
    pub file_path: String,
    pub items_indexed: usize,
    pub items_embedded: usize,
}

pub async fn approve(
    State(state): State<AppState>,
    Path(queue_id): Path<String>,
    body: Option<Json<ApproveBody>>,
) -> Result<Json<ApproveResponse>, ApiError> {
    let body = body.map(|j| j.0).unwrap_or_default();
    let reviewed_by = body.reviewed_by.clone().unwrap_or_else(|| "user".to_string());

    // 1. fetch queue entry
    let entries = state
        .vault
        .store
        .queue_list(None)
        .map_err(ApiError::internal)?;
    let entry = entries
        .into_iter()
        .find(|e| e.id == queue_id)
        .ok_or_else(|| ApiError::not_found(format!("queue entry not found: {queue_id}")))?;
    if entry.status != "pending" {
        return Err(ApiError::bad_request(format!(
            "queue entry not pending (current: {})",
            entry.status
        )));
    }
    if entry.kind != "new" {
        return Err(ApiError::bad_request(format!(
            "approve only supports kind='new' for MVP (got '{}')",
            entry.kind
        )));
    }

    // 2. materialise into a KnowledgeItem with sensible defaults
    let payload: serde_json::Value = serde_json::from_str(&entry.payload_json)
        .map_err(|e| ApiError::bad_request(format!("payload parse: {e}")))?;
    let item = item_from_payload(&payload, &entry.proposed_by);

    // 3. choose target file
    let rel_path = body
        .target_file
        .clone()
        .unwrap_or_else(|| default_target_for(&item, body.scope_path.as_deref()));
    let abs_path = state.vault.root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e))?;
    }

    // 4. ensure the file has a body wrapper, append the section
    ensure_html_file(&abs_path).map_err(|e| ApiError::internal(e))?;
    let section_html = html_emit::emit_section(&item);
    append_before_body_close(&abs_path, &section_html).map_err(|e| ApiError::internal(e))?;

    // 5. reindex that file (with embedder if configured)
    let embedder = state.embedder.as_deref();
    let (items_indexed, items_embedded) = state
        .vault
        .reindex_file(&abs_path, embedder, false)
        .map_err(ApiError::internal)?;

    // 6. mark queue entry approved
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let _ = state.vault.store.update_queue_status(
        &queue_id,
        "approved",
        Some(&reviewed_by),
        Some(&now),
        None,
    );

    Ok(Json(ApproveResponse {
        queue_id,
        item_id: item.id,
        status: "approved".to_string(),
        file_path: rel_path,
        items_indexed,
        items_embedded,
    }))
}

#[derive(Debug, Deserialize, Default)]
pub struct RejectBody {
    #[serde(default)]
    pub reviewed_by: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RejectResponse {
    pub queue_id: String,
    pub status: String,
}

pub async fn reject(
    State(state): State<AppState>,
    Path(queue_id): Path<String>,
    body: Option<Json<RejectBody>>,
) -> Result<Json<RejectResponse>, ApiError> {
    let body = body.map(|j| j.0).unwrap_or_default();
    let reviewed_by = body.reviewed_by.unwrap_or_else(|| "user".to_string());
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    state
        .vault
        .store
        .update_queue_status(
            &queue_id,
            "rejected",
            Some(&reviewed_by),
            Some(&now),
            body.note.as_deref(),
        )
        .map_err(ApiError::internal)?;
    Ok(Json(RejectResponse {
        queue_id,
        status: "rejected".to_string(),
    }))
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn item_from_payload(payload: &serde_json::Value, proposed_by: &str) -> KnowledgeItem {
    let now = OffsetDateTime::now_utc();
    let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    let f = |k: &str| payload.get(k).and_then(|v| v.as_f64()).unwrap_or(0.7);

    KnowledgeItem {
        id: new_id(),
        knowledge_type: s("knowledge_type")
            .map(|t| KnowledgeType::parse(&t))
            .unwrap_or(KnowledgeType::Fact),
        scope: s("scope").map(|t| Scope::parse(&t)).unwrap_or(Scope::User),
        confidence: f("confidence").clamp(0.0, 1.0),
        source_type: s("source_type")
            .map(|t| SourceType::parse(&t))
            .unwrap_or_else(|| {
                if proposed_by.starts_with("agent:") {
                    SourceType::Agent
                } else {
                    SourceType::User
                }
            }),
        status: Status::Active,
        stability: s("stability").map(|t| Stability::parse(&t)).unwrap_or(Stability::MediumTerm),
        created: now,
        modified: now,
        valid_from: None,
        valid_until: None,
        domain: s("domain"),
        tags: payload
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        title: s("title").unwrap_or_default(),
        body_text: payload
            .get("body_text")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                payload
                    .get("body_html")
                    .and_then(|v| v.as_str())
                    .map(|s| strip_html(s))
                    .unwrap_or_default()
            }),
        body_html: payload
            .get("body_html")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default(),
        relations: payload
            .get("relations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| {
                        Some(Relation {
                            rel: x.get("rel")?.as_str()?.to_string(),
                            target: x.get("target")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        claim_key: s("claim_key"),
    }
}

fn default_target_for(item: &KnowledgeItem, scope_path: Option<&str>) -> String {
    let bucket = format!("{}s.html", item.knowledge_type.as_str());
    match item.scope {
        Scope::Project => {
            let proj = scope_path.unwrap_or("default");
            format!("projects/{}/{}", proj, bucket)
        }
        Scope::User => format!("user/{}", bucket),
        Scope::Workspace => format!("workspace/{}", bucket),
        Scope::Organization => format!("organization/{}", bucket),
        Scope::Global => format!("global/{}", bucket),
        Scope::Session => format!("session/{}", bucket),
    }
}

fn ensure_html_file(path: &std::path::Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("vault");
    let initial = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <title>{}</title>\n</head>\n<body>\n\n</body>\n</html>\n",
        title
    );
    std::fs::write(path, initial)
}

fn append_before_body_close(path: &std::path::Path, snippet: &str) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path)?;
    let updated = if let Some(idx) = existing.to_ascii_lowercase().rfind("</body>") {
        let (head, tail) = existing.split_at(idx);
        format!("{}\n{}\n{}", head.trim_end(), snippet, tail)
    } else {
        // No </body> tag: just append.
        format!("{}\n{}\n", existing.trim_end(), snippet)
    };
    std::fs::write(path, updated)
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
