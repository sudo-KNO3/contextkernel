//! ContextKernel storage layer.
//!
//! HTML files under the vault are the source of truth; SQLite + FTS5 is a
//! derived index rebuildable from disk. `Vault::reindex_all()` upserts every
//! `<section data-knowledge-id>` it finds, skipping unchanged items via
//! content-hash short-circuit.

pub mod html_emit;
pub mod html_parse;
pub mod sqlite;
pub mod vault;

pub use sqlite::Store;
pub use vault::Vault;
