//! Query parsing + candidate retrieval.
//!
//! Hybrid: FTS5 candidate pool ∪ top-K semantic neighbours from the
//! in-memory cosine sweep. When the caller passes `anchor_path` (or
//! `anchor_id` — phase 3+), call-graph and folder-proximity signals
//! are folded into the reranker so items defined near or called from
//! the user's working location bubble up.

use crate::rerank;
use ctxk_core::{EmbedderProvider, KnowledgeItem, Result, Scope};
use ctxk_embed::{bytes_to_vec, dot, normalise};
use ctxk_store::{sqlite::ListFilters, Store};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

const SEMANTIC_TOP_K: usize = 50;
const CALL_BFS_MAX_DEPTH: usize = 3;

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
    /// Anchor: path of the file the user is "working on". When set,
    /// rerank adds call-graph proximity + folder proximity signals.
    pub anchor_path: Option<String>,
    /// Anchor by ID; reserved for session 4.
    pub anchor_id: Option<String>,
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
            anchor_path: None,
            anchor_id: None,
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
/// `None` the search degrades to lexical-only.
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

    // ── 3. Anchor context (call-graph + folder proximity) ─────────────────
    let anchor_ctx = build_anchor_context(store, q)?;

    // ── 4. Union pool + per-call filters ──────────────────────────────────
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
    fts_candidates.retain(|it| in_scope_chain(it, &q.scope));
    if let Some(types) = &q.knowledge_types {
        if !types.is_empty() {
            fts_candidates.retain(|it| types.iter().any(|t| t == it.knowledge_type.as_str()));
        }
    }
    if let Some(domains) = &q.domains {
        if !domains.is_empty() {
            fts_candidates.retain(|it| {
                it.domain.as_deref().map(|d| domains.iter().any(|w| w == d)).unwrap_or(false)
            });
        }
    }
    if let Some(tags) = &q.tags_any {
        if !tags.is_empty() {
            fts_candidates.retain(|it| tags.iter().any(|t| it.tags.iter().any(|x| x == t)));
        }
    }

    // ── 5. Rerank + truncate ──────────────────────────────────────────────
    let mut scored: Vec<Scored> = fts_candidates
        .into_iter()
        .map(|item| {
            let in_fts = fts_ids.contains(&item.id);
            let semantic = semantic_scores.get(&item.id).copied().unwrap_or(0.0) as f64;
            let call_proximity = anchor_ctx
                .as_ref()
                .map(|ctx| ctx.call_proximity(&item.id))
                .unwrap_or(0.0);
            let folder_proximity = anchor_ctx
                .as_ref()
                .map(|ctx| ctx.folder_proximity(item.defined_path.as_deref()))
                .unwrap_or(0.0);
            let (score, breakdown) = rerank::score(
                &item, q, semantic, in_fts, call_proximity, folder_proximity,
            );
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

// ─── Anchor context ──────────────────────────────────────────────────────────

struct AnchorContext {
    anchor_path: Option<String>,
    /// distance (1..=CALL_BFS_MAX_DEPTH) from any anchor item to this id;
    /// missing → not within reach.
    call_dist: HashMap<String, usize>,
}

impl AnchorContext {
    fn call_proximity(&self, id: &str) -> f64 {
        match self.call_dist.get(id) {
            None => 0.0,
            Some(0) => 1.0,
            Some(1) => 0.6,
            Some(2) => 0.3,
            Some(_) => 0.1,
        }
    }

    fn folder_proximity(&self, candidate_path: Option<&str>) -> f64 {
        let (Some(a), Some(b)) = (self.anchor_path.as_deref(), candidate_path) else {
            return 0.0;
        };
        if a == b {
            return 1.0;
        }
        let a_segs: Vec<&str> = a.split('/').filter(|s| !s.is_empty()).collect();
        let b_segs: Vec<&str> = b.split('/').filter(|s| !s.is_empty()).collect();
        let common = a_segs.iter().zip(b_segs.iter()).take_while(|(x, y)| x == y).count();
        let max_depth = a_segs.len().max(b_segs.len()).max(1);
        common as f64 / max_depth as f64
    }
}

fn build_anchor_context(store: &Store, q: &Query) -> Result<Option<AnchorContext>> {
    if q.anchor_path.is_none() && q.anchor_id.is_none() {
        return Ok(None);
    }
    // Resolve anchor ID set: union of items at anchor_path + the explicit anchor_id.
    let mut anchor_ids: HashSet<String> = HashSet::new();
    if let Some(p) = &q.anchor_path {
        for id in store.items_at_path(p)? {
            anchor_ids.insert(id);
        }
    }
    if let Some(id) = &q.anchor_id {
        anchor_ids.insert(id.clone());
    }

    // BFS over the call graph (treat calls as undirected for proximity —
    // a function near my caller is just as relevant as one I call).
    let relations = store.all_relations()?;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (src, _rel, dst) in relations {
        adj.entry(src.clone()).or_default().push(dst.clone());
        adj.entry(dst).or_default().push(src);
    }

    let mut dist: HashMap<String, usize> = HashMap::new();
    let mut q_bfs: VecDeque<(String, usize)> = VecDeque::new();
    for id in &anchor_ids {
        dist.insert(id.clone(), 0);
        q_bfs.push_back((id.clone(), 0));
    }
    while let Some((id, d)) = q_bfs.pop_front() {
        if d >= CALL_BFS_MAX_DEPTH {
            continue;
        }
        if let Some(neighbors) = adj.get(&id) {
            for n in neighbors {
                if !dist.contains_key(n) {
                    dist.insert(n.clone(), d + 1);
                    q_bfs.push_back((n.clone(), d + 1));
                }
            }
        }
    }

    Ok(Some(AnchorContext {
        anchor_path: q.anchor_path.clone(),
        call_dist: dist,
    }))
}

// ─── scope helpers ───────────────────────────────────────────────────────────

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
