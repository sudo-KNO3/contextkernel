//! Deterministic rerank scoring with hybrid + anchor signals.
//!
//!     score = 0.45 * semantic         (cosine on normalised embeddings)
//!           + 0.10 * fts_presence     (1.0 if FTS matched, else 0)
//!           + 0.08 * lexical          (jaccard on word sets)
//!           + 0.10 * scope_priority
//!           + 0.08 * recency
//!           + 0.05 * confidence
//!           + 0.02 * source_reliability
//!           + 0.10 * call_proximity   (BFS distance from anchor, 0 if no anchor)
//!           + 0.02 * folder_proximity (shared path prefix with anchor)
//!
//! Without an anchor (`anchor_path` / `anchor_id` unset), the last two
//! terms are zero — score interpretation still works monotonically.

use crate::query::Query;
use ctxk_core::KnowledgeItem;
use serde::Serialize;
use std::collections::HashSet;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Breakdown {
    pub semantic: f64,
    pub fts: f64,
    pub lexical: f64,
    pub scope: f64,
    pub recency: f64,
    pub confidence: f64,
    pub source: f64,
    pub call_proximity: f64,
    pub folder_proximity: f64,
}

pub fn score(
    item: &KnowledgeItem,
    q: &Query,
    semantic: f64,
    in_fts: bool,
    call_proximity: f64,
    folder_proximity: f64,
) -> (f64, Breakdown) {
    let semantic = semantic.clamp(-1.0, 1.0).max(0.0);
    let fts = if in_fts { 1.0 } else { 0.0 };
    let lexical = lexical_overlap(item, &q.task);
    let scope = item.scope.priority();
    let recency = recency_decay(item);
    let confidence = item.confidence.clamp(0.0, 1.0);
    let source = item.source_type.reliability();

    let total = 0.45 * semantic
        + 0.10 * fts
        + 0.08 * lexical
        + 0.10 * scope
        + 0.08 * recency
        + 0.05 * confidence
        + 0.02 * source
        + 0.10 * call_proximity
        + 0.02 * folder_proximity;

    (
        total,
        Breakdown {
            semantic,
            fts,
            lexical,
            scope,
            recency,
            confidence,
            source,
            call_proximity,
            folder_proximity,
        },
    )
}

fn lexical_overlap(item: &KnowledgeItem, task: &str) -> f64 {
    let task_words: HashSet<String> = tokenize(task);
    let item_words: HashSet<String> = tokenize(&format!("{} {}", item.title, item.body_text));
    if task_words.is_empty() || item_words.is_empty() {
        return 0.0;
    }
    let inter = task_words.intersection(&item_words).count() as f64;
    let union = task_words.union(&item_words).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn tokenize(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|t| t.len() > 2)
        .collect()
}

fn recency_decay(item: &KnowledgeItem) -> f64 {
    let now = OffsetDateTime::now_utc();
    let age_secs = (now - item.modified).whole_seconds().max(0) as f64;
    let age_days = age_secs / 86_400.0;
    let halflife = item.stability.halflife_days().max(1.0);
    (-(std::f64::consts::LN_2) * age_days / halflife).exp().clamp(0.0, 1.0)
}
