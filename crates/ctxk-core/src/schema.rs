//! Domain types for ContextKernel knowledge objects.
//!
//! These mirror the `data-*` attributes on the HTML `<section>` elements
//! that live in the vault on disk. See `ctxk-store::html_parse` for the
//! HTML <-> KnowledgeItem round-trip.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Categories of stored knowledge. Drives retrieval filtering and how a
/// fact should be used in a context bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnowledgeType {
    Fact,
    Preference,
    Constraint,
    Assumption,
    Decision,
    Method,
    Formula,
    Source,
    Template,
    Example,
    Task,
    Definition,
    Regulation,
    Warning,
    Relationship,
    /// Catch-all when an HTML attribute uses an unknown value.
    Other,
}

impl KnowledgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Constraint => "constraint",
            Self::Assumption => "assumption",
            Self::Decision => "decision",
            Self::Method => "method",
            Self::Formula => "formula",
            Self::Source => "source",
            Self::Template => "template",
            Self::Example => "example",
            Self::Task => "task",
            Self::Definition => "definition",
            Self::Regulation => "regulation",
            Self::Warning => "warning",
            Self::Relationship => "relationship",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fact" => Self::Fact,
            "preference" => Self::Preference,
            "constraint" => Self::Constraint,
            "assumption" => Self::Assumption,
            "decision" => Self::Decision,
            "method" => Self::Method,
            "formula" => Self::Formula,
            "source" => Self::Source,
            "template" => Self::Template,
            "example" => Self::Example,
            "task" => Self::Task,
            "definition" => Self::Definition,
            "regulation" => Self::Regulation,
            "warning" => Self::Warning,
            "relationship" => Self::Relationship,
            _ => Self::Other,
        }
    }
}

/// Visibility / ownership boundary for a knowledge item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    Session,
    User,
    Project,
    Workspace,
    Organization,
    Global,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::User => "user",
            Self::Project => "project",
            Self::Workspace => "workspace",
            Self::Organization => "organization",
            Self::Global => "global",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "session" => Self::Session,
            "project" => Self::Project,
            "workspace" => Self::Workspace,
            "organization" | "org" => Self::Organization,
            "global" => Self::Global,
            _ => Self::User, // sensible default
        }
    }

    /// Priority weight (1.0 most local / specific, declining outward).
    /// Used by the rerank to favour task-local context.
    pub fn priority(self) -> f64 {
        match self {
            Self::Session => 1.00,
            Self::Project => 0.95,
            Self::User => 0.80,
            Self::Workspace => 0.70,
            Self::Organization => 0.55,
            Self::Global => 0.40,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Active,
    Stale,
    Superseded,
    Deleted,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "stale" => Self::Stale,
            "superseded" => Self::Superseded,
            "deleted" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stability {
    Temporary,
    ShortTerm,
    MediumTerm,
    LongTerm,
    Permanent,
}

impl Stability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Temporary => "temporary",
            Self::ShortTerm => "short-term",
            Self::MediumTerm => "medium-term",
            Self::LongTerm => "long-term",
            Self::Permanent => "permanent",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "temporary" => Self::Temporary,
            "short-term" => Self::ShortTerm,
            "long-term" => Self::LongTerm,
            "permanent" => Self::Permanent,
            _ => Self::MediumTerm,
        }
    }

    /// Recency half-life in days for the rerank decay function.
    pub fn halflife_days(self) -> f64 {
        match self {
            Self::Temporary => 1.0,
            Self::ShortTerm => 14.0,
            Self::MediumTerm => 90.0,
            Self::LongTerm => 365.0,
            Self::Permanent => 36500.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    User,
    Document,
    Agent,
    Imported,
    Cited,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Document => "document",
            Self::Agent => "agent",
            Self::Imported => "imported",
            Self::Cited => "cited",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "document" | "doc" => Self::Document,
            "agent" | "ai" => Self::Agent,
            "imported" | "import" => Self::Imported,
            "cited" | "citation" | "cited-source" => Self::Cited,
            _ => Self::User,
        }
    }

    /// Source reliability prior used in rerank. Higher = trust more.
    pub fn reliability(self) -> f64 {
        match self {
            Self::User => 1.0,
            Self::Cited => 0.9,
            Self::Document => 0.8,
            Self::Imported => 0.7,
            Self::Agent => 0.5,
        }
    }
}

/// A typed inter-item relationship. Modelled as `<span data-rel data-target>`
/// inside the `<section>` body.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Relation {
    /// Relationship kind: `supersedes`, `cites`, `contradicts`, `depends-on`,
    /// `refines`, `example-of`, …
    pub rel: String,
    /// Target ULID of the related item. May reference a not-yet-loaded item.
    pub target: String,
}

/// The canonical in-memory representation of one knowledge object.
///
/// Round-trips losslessly with the `<section data-*>` HTML on disk; the
/// HTML file is the source of truth and SQLite is a derived index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeItem {
    pub id: String, // ULID
    pub knowledge_type: KnowledgeType,
    pub scope: Scope,
    pub confidence: f64,
    pub source_type: SourceType,
    pub status: Status,
    pub stability: Stability,

    #[serde(with = "time::serde::rfc3339")]
    pub created: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub modified: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub valid_from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub valid_until: Option<OffsetDateTime>,

    pub domain: Option<String>,
    pub tags: Vec<String>,

    pub title: String,
    pub body_text: String,
    pub body_html: String,

    pub relations: Vec<Relation>,

    /// Optional explicit grouping key for conflict detection. If absent,
    /// the retrieval engine derives one from title + domain.
    pub claim_key: Option<String>,

    // ── Code-aware fields (added by ctxk-code). All optional so non-code
    //    items (regular knowledge) keep the original on-disk shape. ────────
    /// Vault-relative path of the source file this item was extracted from
    /// (e.g. `crates/ctxk-store/src/sqlite.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_end_line: Option<usize>,
}

impl KnowledgeItem {
    /// Compute the conflict-detection grouping key.
    pub fn derived_claim_key(&self) -> String {
        if let Some(k) = &self.claim_key {
            return k.clone();
        }
        let title_slug = slugify(&self.title);
        let domain = self.domain.as_deref().unwrap_or("");
        if domain.is_empty() {
            title_slug
        } else {
            format!("{}:{}", title_slug, domain)
        }
    }
}

/// Lightweight slug: lowercase, alphanumerics + `-`, runs collapsed.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = true;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}
