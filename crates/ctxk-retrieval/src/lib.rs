//! ContextKernel retrieval — query, rerank, conflict, bundle.

pub mod bundle;
pub mod conflict;
pub mod query;
pub mod rerank;

pub use bundle::{assemble, ContextBundle};
pub use query::{execute, Query, Scored};
