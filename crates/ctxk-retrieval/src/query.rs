//! Query parsing + candidate retrieval.
//!
//! Hybrid: FTS5 candidate pool ∪ top-K semantic neighbours from the
//! in-memory cosine sweep. The reranker then combines per-item signals.

use crate::rerank;
use ctxk_core::{EmbedderProvider, KnowledgeItem, Result, Scope};
use ctxk_embed::{bytes_to_vec, dot, normalise};
use ctxk_store::{sqlite::ListFilters, Store};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const SEMANTIC_TOP_K: usize = 50;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Query {
    pub task: String,
    pub scope: Option<String>,
    pub scope_path: Option<String>,
    pub knowledge_types: Option<Vec<String>>,
    pub domains: Option<Vec<String>>,
    pub tags_any: Option<Vec<String>>,
    pub include_stale: bool,
    pub max_items: usize,
    pub include_conflicts: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            task: String::new(),
            scope: None,
            scope_path: None,
            knowledge_types: None,
            domains: None,
            tags_any: None,
            include_stale: false,
            max_items: 12,
            include_conflicts: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Scored {
    pub item: KnowledgeItem,
    pub score: f64,
    pub score_breakdown: rerank::Breakdown,
}

/// Execute the query with an optional embedder. When the embedder is
/// `None` the search degrades to lexical-only (session-1 behaviour).
pub fn execute(
    store: &Store,
    q: &Query,
    embedder: Option<&dyn EmbedderProvider>,
) -> Result<Vec<Scored>> {
    let _ = store.sweep_stale();

    // ── 1. FTS candidate pool ─────────────────────────────────────────────
    let fts_query = if q.task.trim().is_empty() {
        None
    } else {
        Some(sanitise_fts(&q.task))
    };
    let include_status = if q.include_stale {
        vec![
            "active".to_string(),
            "stale".to_string(),
            "superseded".to_string(),
        ]
    } else {
        vec!["active".to_string()]
    };
    let pool_size = (q.max_items.max(1) * 5).min(200);
    let filters = ListFilters {
        scope: None,
        knowledge_type: None,
        domain: None,
        tag: None,
        q: fts_query.clone(),
        include_status: include_status.clone(),
        limit: Some(pool_size as i64),
        offset: None,
    };
    let mut fts_candidates = store.list_items(&filters)?;
    let fts_ids: HashSet<String> = fts_candidates.iter().map(|i| i.id.clone()).collect();

    // ── 2. Semantic candidate pool ────────────────────────────────────────
    let mut semantic_scores: HashMap<String, f32> = HashMap::new();
    if let (Some(em), false) = (embedder, q.task.trim().is_empty()) {
        let mut query_vec = em.embed_batch(&[q.task.clone()])?;
        if let Some(mut q_emb) = query_vec.pop() {
            normalise(&mut q_emb);
            let status_refs: Vec<&str> = include_status.iter().map(|s| s.as_str()).collect();
            let all_embeds = store.list_embeddings(None, &status_refs)?;
            let mut topk: Vec<(String, f32)> = all_embeds
                .into_iter()
                .map(|(id, bytes)| {
                    let v = bytes_to_vec(&bytes);
                    let s = if v.len() == q_emb.len() { dot(&v, &q_emb) } else { 0.0 };
                    (id, s)
                })
                .collect();
            topk.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            topk.truncate(SEMANTIC_TOP_K);
            for (id, score) in topk {
                semantic_scores.insert(id, score);
            }
        }
    }

    // ── 3. Union pool + per-call filters ──────────────────────────────────
    // Fetch any semantic-only candidates that weren't already in FTS.
    let extra_ids: Vec<String> = semantic_scores
        .keys()
        .filter(|id| !fts_ids.contains(id.as_str()))
        .cloned()
        .collect();
    for id in &extra_ids {
        if let Ok(Some(it)) = store.get_item(id) {
            fts_candidates.push(it);
        }
    }
    // Filters: scope chain + type / domain / tags-any.
    fts_candidates.retain(|it| in_scope_chain(it, &q.scope));
    if let Some(types) = &q.knowledge_types {
        if !types.is_empty() {
            fts_candidates.retain(|it| types.iter().any(|t| t == it.knowledge_type.as_str()));
        }
    }
    if let Some(domains) = &q.domains {
        if !domains.is_empty() {
            fts_candidates.retain(|it| {
                it.domain
                    .as_deref()
                    .map(|d| domains.iter().any(|w| w == d))
                    .unwrap_or(false)
            });
        }
    }
    if let Some(tags) = &q.tags_any {
        if !tags.is_empty() {
            fts_candidates.retain(|it| {
                tags.iter().any(|t| it.tags.iter().any(|x| x == t))
            });
        }
    }

    // ── 4. Rerank + truncate ──────────────────────────────────────────────
    let mut scored: Vec<Scored> = fts_candidates
        .into_iter()
        .map(|item| {
            let in_fts = fts_ids.contains(&item.id);
            let semantic = semantic_scores.get(&item.id).copied().unwrap_or(0.0) as f64;
            let (score, breakdown) = rerank::score(&item, q, semantic, in_fts);
            Scored {
                item,
                score,
                score_breakdown: breakdown,
            }
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(q.max_items.max(1));
    Ok(scored)
}

fn in_scope_chain(item: &KnowledgeItem, requested: &Option<String>) -> bool {
    let Some(req) = requested else {
        return true;
    };
    let req_scope = Scope::parse(req);
    scope_chain(req_scope).iter().any(|s| *s == item.scope)
}

fn scope_chain(s: Scope) -> Vec<Scope> {
    match s {
        Scope::Session => vec![
            Scope::Session,
            Scope::Project,
            Scope::User,
            Scope::Workspace,
            Scope::Organization,
            Scope::Global,
        ],
        Scope::Project => vec![
            Scope::Project,
            Scope::User,
            Scope::Workspace,
            Scope::Organization,
            Scope::Global,
        ],
        Scope::User => vec![Scope::User, Scope::Workspace, Scope::Organization, Scope::Global],
        Scope::Workspace => vec![Scope::Workspace, Scope::Organization, Scope::Global],
        Scope::Organization => vec![Scope::Organization, Scope::Global],
        Scope::Global => vec![Scope::Global],
    }
}

fn sanitise_fts(s: &str) -> String {
    s.split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}
