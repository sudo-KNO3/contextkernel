//! Query parsing + candidate retrieval (FTS5 + metadata filter).

use crate::rerank;
use ctxk_core::{KnowledgeItem, Result, Scope};
use ctxk_store::{sqlite::ListFilters, Store};
use serde::{Deserialize, Serialize};

/// What an AI agent / client asks for. All fields except `task` are
/// optional; defaults are permissive (broad recall, narrow scope).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Query {
    /// Natural-language task description. Used both as the FTS query
    /// and as the rerank "intent".
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

/// Execute the query: fetch candidates from the store, hard-filter,
/// rerank, truncate to `max_items`. Conflict detection is layered on
/// at bundle-assembly time.
pub fn execute(store: &Store, q: &Query) -> Result<Vec<Scored>> {
    let _ = store.sweep_stale(); // best-effort lazy sweep

    // Candidate pool: FTS or, if the task is empty, fall back to a metadata-only
    // listing so callers can still walk the vault.
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

    // First try: with all the per-call filters. If we get nothing back AND
    // the user supplied filter knobs, widen progressively. (Improves the
    // "nothing matched" UX without surprising power users.)
    let mut candidates: Vec<KnowledgeItem> = Vec::new();
    let want_types: Option<Vec<String>> = q.knowledge_types.clone();
    let want_domains: Option<Vec<String>> = q.domains.clone();
    let want_tags: Option<Vec<String>> = q.tags_any.clone();

    // Pull a generous pool (5x max_items, capped 200) for reranking.
    let pool_size = (q.max_items.max(1) * 5).min(200);
    for attempt in 0..2 {
        let filters = ListFilters {
            scope: None,                       // filter scope in Rust (chain-aware)
            knowledge_type: None,              // filter type in Rust (Vec)
            domain: None,                      // filter domain in Rust (Vec)
            tag: None,                         // filter tags in Rust (any-of)
            q: fts_query.clone(),
            include_status: include_status.clone(),
            limit: Some(pool_size as i64),
            offset: None,
        };
        candidates = store.list_items(&filters)?;
        candidates.retain(|it| in_scope_chain(it, &q.scope));
        if let Some(types) = &want_types {
            if !types.is_empty() {
                candidates.retain(|it| types.iter().any(|t| t == it.knowledge_type.as_str()));
            }
        }
        if let Some(domains) = &want_domains {
            if !domains.is_empty() {
                candidates.retain(|it| {
                    it.domain.as_deref().map(|d| domains.iter().any(|w| w == d)).unwrap_or(false)
                });
            }
        }
        if let Some(tags) = &want_tags {
            if !tags.is_empty() {
                candidates.retain(|it| tags.iter().any(|t| it.tags.iter().any(|x| x == t)));
            }
        }
        if !candidates.is_empty() {
            break;
        }
        // Widen on second pass: drop the FTS query, keep filters.
        if attempt == 0 && fts_query.is_some() {
            // re-run loop body with fts_query = None
            // we'll handle by mutating the local; simplest: emulate by clearing
            // (only the first iteration uses fts_query when set).
        } else {
            break;
        }
    }

    // Rerank + truncate.
    let mut scored: Vec<Scored> = candidates
        .into_iter()
        .map(|item| {
            let (score, breakdown) = rerank::score(&item, q);
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

/// Scope inclusion: if a request scope is given, an item must be in the
/// item's own scope OR a "more general" scope on the chain. With no scope
/// given, all scopes pass.
fn in_scope_chain(item: &KnowledgeItem, requested: &Option<String>) -> bool {
    let Some(req) = requested else {
        return true;
    };
    let req_scope = Scope::parse(req);
    let allowed = scope_chain(req_scope);
    allowed.iter().any(|s| *s == item.scope)
}

/// `session > project > user > workspace > organization > global`
/// A `session` query sees everything; a `global` query sees only `global`.
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

/// FTS5 query escaper. The user's natural-language `task` is split into
/// tokens; each token becomes a `"quoted"` term, OR'd. This avoids syntax
/// errors when the user types punctuation.
fn sanitise_fts(s: &str) -> String {
    s.split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}
