//! Deterministic rerank scoring.
//!
//! Session-1 mix (no semantic embeddings yet):
//!     score = 0.45 * fts_proxy        (presence of any task token in body/title)
//!           + 0.25 * lexical_overlap  (jaccard-style on word sets)
//!           + 0.10 * scope_priority
//!           + 0.08 * recency
//!           + 0.07 * confidence
//!           + 0.05 * source_reliability
//!
//! Session 2 will replace `fts_proxy + lexical_overlap` with a cosine
//! similarity term sourced from the embedding store.

use crate::query::Query;
use ctxk_core::KnowledgeItem;
use serde::Serialize;
use std::collections::HashSet;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize)]
pub struct Breakdown {
    pub fts: f64,
    pub lexical: f64,
    pub scope: f64,
    pub recency: f64,
    pub confidence: f64,
    pub source: f64,
}

pub fn score(item: &KnowledgeItem, q: &Query) -> (f64, Breakdown) {
    let fts = fts_proxy(item, &q.task);
    let lexical = lexical_overlap(item, &q.task);
    let scope = item.scope.priority();
    let recency = recency_decay(item);
    let confidence = item.confidence.clamp(0.0, 1.0);
    let source = item.source_type.reliability();

    let total = 0.45 * fts
        + 0.25 * lexical
        + 0.10 * scope
        + 0.08 * recency
        + 0.07 * confidence
        + 0.05 * source;

    (
        total,
        Breakdown {
            fts,
            lexical,
            scope,
            recency,
            confidence,
            source,
        },
    )
}

fn fts_proxy(item: &KnowledgeItem, task: &str) -> f64 {
    if task.trim().is_empty() {
        return 0.0;
    }
    let hay = format!("{} {}", item.title, item.body_text).to_ascii_lowercase();
    let tokens: Vec<&str> = task
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let hits = tokens
        .iter()
        .filter(|t| hay.contains(&t.to_ascii_lowercase()))
        .count();
    (hits as f64) / (tokens.len() as f64)
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
    // exp(-ln(2) * age / halflife) → 1.0 at age=0, 0.5 at half-life
    (-(std::f64::consts::LN_2) * age_days / halflife).exp().clamp(0.0, 1.0)
}
