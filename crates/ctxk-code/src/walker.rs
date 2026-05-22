//! Walk a code root, dispatch per-file parsing, and write the resulting
//! KnowledgeItems into the vault as HTML files (one per source file).

use crate::{rust::parse_rust, CodeSymbol};
use ctxk_core::{
    new_id, KnowledgeItem, KnowledgeType, Relation, Result, Scope, SourceType, Stability, Status,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "venv",
    ".venv",
    "__pycache__",
    ".cargo",
    "_smoke_vault",
    ".fastembed_cache",
];

#[derive(Debug, Clone, Default)]
pub struct IndexReport {
    pub files_parsed: usize,
    pub symbols_emitted: usize,
    pub files_written: usize,
    pub errors: Vec<(PathBuf, String)>,
}

pub struct CodeIndexer<'a> {
    pub project_name: &'a str,
    pub project_root: &'a Path,
    pub vault_root: &'a Path,
}

impl<'a> CodeIndexer<'a> {
    /// Walk `project_root`, parse every source file we support, write one
    /// HTML file per source under `vault_root/code/<project>/<rel>.html`.
    /// Returns counts + a list of vault-relative HTML files written, so
    /// the caller can reindex just those files.
    pub fn run(&self) -> Result<(IndexReport, Vec<PathBuf>)> {
        let mut report = IndexReport::default();
        let mut written: Vec<PathBuf> = Vec::new();
        // Group symbols by source file path.
        let mut by_file: HashMap<String, Vec<CodeSymbol>> = HashMap::new();

        for entry in WalkDir::new(self.project_root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !SKIP_DIRS.iter().any(|s| *s == name)
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext != "rs" {
                // Phase 1: Rust only. Python / TS / JS in the next pass.
                continue;
            }
            report.files_parsed += 1;
            let rel = relative_posix(path, self.project_root);
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    report.errors.push((path.to_path_buf(), e.to_string()));
                    continue;
                }
            };
            let symbols = parse_rust(&src, &rel);
            by_file.entry(rel).or_default().extend(symbols);
        }

        // Resolve calls/imports to ULIDs by name. The "name" we resolve on is
        // the symbol `name` — collisions (two `new()` methods on different
        // structs) are unavoidable at this granularity; rerank's call_proximity
        // still benefits from any-of matching.
        let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();
        let mut symbol_ids: HashMap<(String, String, usize), String> = HashMap::new(); // (file, name, start_line) → ULID
        for syms in by_file.values() {
            for s in syms {
                let id = new_id();
                symbol_ids.insert((s.file_path.clone(), s.name.clone(), s.start_line), id.clone());
                name_to_ids.entry(s.name.clone()).or_default().push(id);
            }
        }

        // Materialise items + write HTML files.
        for (file_path, syms) in by_file {
            report.symbols_emitted += syms.len();
            let html = emit_file_html(self.project_name, &file_path, &syms, &symbol_ids, &name_to_ids);
            let out_rel = code_html_path(self.project_name, &file_path);
            let out_abs = self.vault_root.join(&out_rel);
            if let Some(parent) = out_abs.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out_abs, html)?;
            report.files_written += 1;
            written.push(out_abs);
        }

        Ok((report, written))
    }
}

fn code_html_path(project: &str, src_rel: &str) -> PathBuf {
    // Replace path separators in the source path with `__` so each source
    // file maps to a single flat HTML file. Cleaner than mirroring deep
    // directory structures in the vault.
    let flat = src_rel.replace('/', "__").replace('\\', "__");
    PathBuf::from(format!("code/{}/{}.html", project, flat))
}

fn emit_file_html(
    project: &str,
    src_path: &str,
    symbols: &[CodeSymbol],
    symbol_ids: &HashMap<(String, String, usize), String>,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> String {
    let mut s = String::new();
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <title>");
    s.push_str(&escape_html(project));
    s.push_str(" / ");
    s.push_str(&escape_html(src_path));
    s.push_str("</title>\n</head>\n<body>\n\n");

    for sym in symbols {
        let id = symbol_ids
            .get(&(sym.file_path.clone(), sym.name.clone(), sym.start_line))
            .cloned()
            .unwrap_or_else(new_id);

        let item = symbol_to_item(&id, project, sym, name_to_ids);
        s.push_str(&ctxk_store::html_emit::emit_section(&item));
        s.push('\n');
    }

    s.push_str("</body>\n</html>\n");
    s
}

fn symbol_to_item(
    id: &str,
    project: &str,
    sym: &CodeSymbol,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> KnowledgeItem {
    let now = OffsetDateTime::now_utc();
    let title = if sym.qualified_name == sym.name {
        sym.name.clone()
    } else {
        sym.qualified_name.clone()
    };

    // body_text feeds embeddings + FTS: title, file path, full source.
    let body_text = format!("{}\n{}\n{}", title, sym.file_path, sym.body);

    // body_html: <pre><code> wrap for human rendering.
    let body_html = format!(
        "  <h3>{} <small>{}:{}-{}</small></h3>\n  <pre><code class=\"language-{}\">{}</code></pre>\n",
        escape_html(&title),
        escape_html(&sym.file_path),
        sym.start_line,
        sym.end_line,
        sym.language,
        escape_html(&sym.body)
    );

    // Relations: resolve call-name and import-path strings to known IDs.
    let mut relations: Vec<Relation> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for callee in &sym.calls {
        if let Some(targets) = name_to_ids.get(callee) {
            for t in targets {
                if t == id {
                    continue;
                } // self-recursion, skip
                let key = ("calls".into(), t.clone());
                if seen.insert(key.clone()) {
                    relations.push(Relation {
                        rel: key.0,
                        target: key.1,
                    });
                }
            }
        }
    }
    for imp in &sym.imports {
        // imports are external; we store them as relations to a synthetic
        // "external-name" target. They still help rerank when an import
        // mentions a function name that IS in the vault.
        let leaf = imp.rsplit_once("::").map(|(_, l)| l).unwrap_or(imp);
        if let Some(targets) = name_to_ids.get(leaf) {
            for t in targets {
                let key = ("uses".into(), t.clone());
                if seen.insert(key.clone()) {
                    relations.push(Relation {
                        rel: key.0,
                        target: key.1,
                    });
                }
            }
        }
    }

    KnowledgeItem {
        id: id.to_string(),
        knowledge_type: KnowledgeType::parse(sym.kind.knowledge_type()),
        scope: Scope::Project,
        confidence: 1.0,
        source_type: SourceType::Imported,
        status: Status::Active,
        stability: Stability::LongTerm,
        created: now,
        modified: now,
        valid_from: None,
        valid_until: None,
        domain: Some(format!("code/{}", sym.language)),
        tags: vec![
            "code".into(),
            sym.language.clone(),
            sym.kind.as_str().into(),
            project.into(),
        ],
        title,
        body_text,
        body_html,
        relations,
        claim_key: Some(format!("{}#{}", sym.file_path, sym.qualified_name)),
        defined_path: Some(sym.file_path.clone()),
        defined_start_line: Some(sym.start_line),
        defined_end_line: Some(sym.end_line),
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn relative_posix(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
