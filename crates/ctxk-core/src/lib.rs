//! ContextKernel core types — no I/O, no async, no engines.
//!
//! Everything downstream depends on these enums and the [`KnowledgeItem`]
//! struct as the lingua franca between the store, retrieval, and server crates.

pub mod error;
pub mod id;
pub mod schema;

pub use error::{Error, Result};
pub use id::new_id;
pub use schema::{KnowledgeItem, KnowledgeType, Relation, Scope, SourceType, Stability, Status};
