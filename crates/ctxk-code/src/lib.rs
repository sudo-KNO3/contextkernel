//! Source-code ingestion for ContextKernel.
//!
//! Walks a code root, parses each file with tree-sitter, extracts top-level
//! definitions (functions, structs, impls, traits) as [`CodeSymbol`]s
//! along with their outgoing call-sites and `use` imports. Symbols are
//! then materialised as `KnowledgeItem`s with new attributes:
//!
//! - `data-path`           — POSIX path of the source file under `project_root`
//! - `data-defined-at`     — "start-end" line range of the symbol body
//! - `data-source-type`    — `code-import`
//! - `data-knowledge-type` — `method` (functions/methods), `definition`
//!                            (structs/traits)
//!
//! Calls and imports become `<span data-rel data-target>` edges. Targets
//! are by-name first; resolution to other items happens at retrieval time
//! through a name→id lookup table the caller maintains in SQLite.

pub mod rust;
pub mod walker;

use serde::{Deserialize, Serialize};

/// Kind of source-level symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    Function,
    Struct,
    Impl,
    Trait,
    Enum,
    Module,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Impl => "impl",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Module => "module",
        }
    }
    pub fn knowledge_type(self) -> &'static str {
        match self {
            Self::Function => "method",
            _ => "definition",
        }
    }
}

/// One top-level definition extracted from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub name: String,
    pub qualified_name: String, // module::path::name
    pub kind: SymbolKind,
    pub file_path: String,      // POSIX, relative to project root
    pub start_line: usize,      // 1-based, inclusive
    pub end_line: usize,        // 1-based, inclusive
    /// Verbatim source text of the symbol (used as body_text for embedding
    /// + body_html for rendering, after `<pre><code>` wrapping).
    pub body: String,
    /// Function names this symbol calls directly. By-name only at parse
    /// time; the store resolves them to IDs.
    pub calls: Vec<String>,
    /// Paths in `use` statements that affected this file (whole-file
    /// duplicates across symbols are fine — easier than scoping).
    pub imports: Vec<String>,
    /// Language identifier (rust, python, ...). One language per call to
    /// the parser for now.
    pub language: String,
}

impl CodeSymbol {
    /// Slug used in `<code class="language-…">` and in derived `claim_key`.
    pub fn language_slug(&self) -> &str {
        &self.language
    }
}
