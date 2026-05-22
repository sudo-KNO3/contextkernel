//! High-level Vault: bind a root directory to a [`Store`] and walk it.

use crate::{html_parse, sqlite::Store};
use ctxk_core::{EmbedderProvider, KnowledgeItem, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const SYSTEM_DIRNAME: &str = "system";
pub const INDEX_FILENAME: &str = "index.sqlite";

pub struct Vault {
    pub root: PathBuf,
    pub store: Store,
}

impl Vault {
    /// Open an existing vault, creating `system/index.sqlite` if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let db_path = root.join(SYSTEM_DIRNAME).join(INDEX_FILENAME);
        let store = Store::open(db_path)?;
        Ok(Self { root, store })
    }

    /// Initialise a brand-new vault directory layout. Creates standard
    /// subdirectories (user/, projects/, domains/, templates/, sources/,
    /// system/). Idempotent.
    pub fn init(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for sub in [
            "user",
            "projects",
            "domains",
            "templates",
            "sources",
            SYSTEM_DIRNAME,
        ] {
            std::fs::create_dir_all(root.join(sub))?;
        }
        Self::open(root)
    }

    /// Walk the entire vault and (re)index every `.html` file. When an
    /// embedder is provided, items whose body text changed get re-embedded
    /// in the same pass.
    pub fn reindex_all(&self, embedder: Option<&dyn EmbedderProvider>) -> Result<ReindexReport> {
        let mut files_scanned = 0_usize;
        let mut items_indexed = 0_usize;
        let mut items_embedded = 0_usize;
        let mut files_skipped = 0_usize;
        let mut errors: Vec<(PathBuf, String)> = Vec::new();

        // Detect embedding-model change → force full re-embed.
        let mut force_embed = false;
        if let Some(em) = embedder {
            let stored = self.store.get_meta("embedding_model").unwrap_or(None);
            if stored.as_deref() != Some(em.model_name()) {
                tracing::info!(
                    "embedding model changed ({:?} -> {}); re-embedding everything",
                    stored,
                    em.model_name()
                );
                force_embed = true;
                let _ = self
                    .store
                    .set_meta("embedding_model", em.model_name());
                let _ = self
                    .store
                    .set_meta("embedding_dim", &em.dim().to_string());
            }
        }

        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| {
                !e.path()
                    .iter()
                    .any(|c| c.to_string_lossy() == SYSTEM_DIRNAME)
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("html") {
                continue;
            }
            files_scanned += 1;
            match self.reindex_file(path, embedder, force_embed) {
                Ok((n, e)) => {
                    if n == 0 {
                        files_skipped += 1;
                    } else {
                        items_indexed += n;
                        items_embedded += e;
                    }
                }
                Err(e) => errors.push((path.to_path_buf(), e.to_string())),
            }
        }

        let _ = self.store.sweep_stale();

        Ok(ReindexReport {
            files_scanned,
            items_indexed,
            items_embedded,
            files_skipped,
            errors,
        })
    }

    /// Reindex one file. Returns (items_indexed, items_embedded).
    pub fn reindex_file(
        &self,
        path: &Path,
        embedder: Option<&dyn EmbedderProvider>,
        force_embed: bool,
    ) -> Result<(usize, usize)> {
        let rel = relative_posix(path, &self.root);
        let items = html_parse::parse_file(path)?;
        // Replace strategy: simpler than diffing for MVP.
        self.store.delete_items_for_file(&rel)?;
        for item in &items {
            self.store.upsert_item(item, &rel, 0)?;
        }

        let mut embedded = 0_usize;
        if let Some(em) = embedder {
            embedded = self.embed_items(&items, em, force_embed)?;
        }
        Ok((items.len(), embedded))
    }

    /// Compute and persist embeddings for a batch of items. Skips items
    /// that already have an embedding unless `force` is set.
    pub fn embed_items(
        &self,
        items: &[KnowledgeItem],
        embedder: &dyn EmbedderProvider,
        force: bool,
    ) -> Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        // Filter to items that need embedding.
        let mut to_embed: Vec<(&KnowledgeItem, String)> = Vec::new();
        for it in items {
            if !force {
                let has = self.store.get_embedding(&it.id)?.is_some();
                if has {
                    continue;
                }
            }
            let text = embed_text_for(it);
            to_embed.push((it, text));
        }
        if to_embed.is_empty() {
            return Ok(0);
        }

        // Batch through the embedder.
        let texts: Vec<String> = to_embed.iter().map(|(_, t)| t.clone()).collect();
        let vectors = embedder.embed_batch(&texts)?;
        if vectors.len() != to_embed.len() {
            return Err(ctxk_core::Error::Other(format!(
                "embed_batch length mismatch: {} in, {} out",
                to_embed.len(),
                vectors.len()
            )));
        }
        for ((it, _), mut v) in to_embed.into_iter().zip(vectors.into_iter()) {
            // Normalise so cosine collapses to a dot product downstream.
            normalise(&mut v);
            let bytes = vec_to_bytes(&v);
            self.store.set_embedding(&it.id, &bytes)?;
        }
        Ok(texts.len())
    }
}

/// Pull the text used to compute the embedding for an item. Concatenates
/// title and body_text — gives the title some natural extra weight without
/// over-engineering tokenisation.
fn embed_text_for(item: &KnowledgeItem) -> String {
    if item.title.is_empty() {
        item.body_text.clone()
    } else {
        format!("{}\n\n{}", item.title, item.body_text)
    }
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf
}

fn normalise(v: &mut [f32]) {
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        for x in v.iter_mut() {
            *x /= mag;
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReindexReport {
    pub files_scanned: usize,
    pub items_indexed: usize,
    pub items_embedded: usize,
    pub files_skipped: usize,
    pub errors: Vec<(PathBuf, String)>,
}

fn relative_posix(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
