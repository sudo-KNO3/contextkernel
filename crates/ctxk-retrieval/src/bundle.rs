//! Top-level "context bundle" assembly — the response shape that AI
//! agents receive from `POST /context/query`.

use crate::{conflict::Conflict, query::Query, query::Scored};
use ctxk_core::{KnowledgeItem, Result};
use ctxk_store::Store;
use serde::Serialize;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Serialize)]
pub struct BundleItem {
    pub id: String,
    pub score: f64,
    pub knowledge_type: String,
    pub scope: String,
    pub title: String,
    pub body_html: String,
    pub body_text: String,
    pub confidence: f64,
    pub source_type: String,
    pub status: String,
    pub stability: String,
    pub created: String,
    pub modified: String,
    pub valid_until: Option<String>,
    pub domain: Option<String>,
    pub tags: Vec<String>,
    pub relations: Vec<RelationView>,
    pub score_breakdown: crate::rerank::Breakdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationView {
    pub rel: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextBundle {
    pub query_id: String,
    pub items: Vec<BundleItem>,
    pub conflicts: Vec<Conflict>,
    pub total_candidates: usize,
    pub stale_excluded: usize,
}

pub fn assemble(store: &Store, q: &Query, scored: Vec<Scored>) -> Result<ContextBundle> {
    let total_candidates = scored.len();
    let conflicts = if q.include_conflicts {
        crate::conflict::detect(&scored)
    } else {
        Vec::new()
    };

    let items: Vec<BundleItem> = scored
        .into_iter()
        .map(|s| BundleItem {
            id: s.item.id.clone(),
            score: round(s.score, 4),
            knowledge_type: s.item.knowledge_type.as_str().to_string(),
            scope: s.item.scope.as_str().to_string(),
            title: s.item.title.clone(),
            body_html: s.item.body_html.clone(),
            body_text: s.item.body_text.clone(),
            confidence: s.item.confidence,
            source_type: s.item.source_type.as_str().to_string(),
            status: s.item.status.as_str().to_string(),
            stability: s.item.stability.as_str().to_string(),
            created: s.item.created.format(&Rfc3339).unwrap_or_default(),
            modified: s.item.modified.format(&Rfc3339).unwrap_or_default(),
            valid_until: s.item.valid_until.and_then(|d| d.format(&Rfc3339).ok()),
            domain: s.item.domain.clone(),
            tags: s.item.tags.clone(),
            relations: s
                .item
                .relations
                .iter()
                .map(|r| RelationView {
                    rel: r.rel.clone(),
                    target: r.target.clone(),
                })
                .collect(),
            score_breakdown: s.score_breakdown,
        })
        .collect();

    // Stale-excluded count is the difference between candidates that matched
    // the query at the store layer vs items returned. We don't track this
    // exactly in MVP — surface the stats endpoint count instead, and report
    // zero here unless `include_stale` was false AND we have data.
    let stale_excluded = if !q.include_stale {
        // Best-effort: total - kept. Will refine in session 2.
        0
    } else {
        0
    };
    let _ = store; // store retained for the signature; future use for graph expansion

    Ok(ContextBundle {
        query_id: ctxk_core::new_id(),
        items,
        conflicts,
        total_candidates,
        stale_excluded,
    })
}

fn round(x: f64, decimals: u32) -> f64 {
    let m = 10f64.powi(decimals as i32);
    (x * m).round() / m
}

/// Convenience: also expose the raw item for callers that want it.
#[allow(dead_code)]
pub fn into_items(scored: Vec<Scored>) -> Vec<KnowledgeItem> {
    scored.into_iter().map(|s| s.item).collect()
}
