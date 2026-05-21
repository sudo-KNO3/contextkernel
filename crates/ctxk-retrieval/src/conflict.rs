//! Conflict detection: group active items by (scope, claim_key).
//! Any group with >1 active item is flagged.

use crate::query::Scored;
use ctxk_core::Status;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Conflict {
    pub id: String,           // synthesised ULID for this conflict instance
    pub claim_key: String,
    pub scope: String,
    pub item_ids: Vec<String>,
}

pub fn detect(scored: &[Scored]) -> Vec<Conflict> {
    let mut buckets: HashMap<(String, String), Vec<String>> = HashMap::new();
    for s in scored {
        if s.item.status != Status::Active {
            continue;
        }
        let k = (s.item.scope.as_str().to_string(), s.item.derived_claim_key());
        buckets.entry(k).or_default().push(s.item.id.clone());
    }
    buckets
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|((scope, claim_key), item_ids)| Conflict {
            id: ctxk_core::new_id(),
            claim_key,
            scope,
            item_ids,
        })
        .collect()
}
