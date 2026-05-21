//! High-level Vault: bind a root directory to a [`Store`] and walk it.

use crate::{html_parse, sqlite::Store};
use ctxk_core::Result;
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

    /// Walk the entire vault and (re)index every `.html` file. Returns
    /// `(files_scanned, items_indexed, files_skipped_unchanged)`.
    pub fn reindex_all(&self) -> Result<ReindexReport> {
        let mut files_scanned = 0_usize;
        let mut items_indexed = 0_usize;
        let mut files_skipped = 0_usize;
        let mut errors: Vec<(PathBuf, String)> = Vec::new();

        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| {
                // Skip the system/ dir — it holds the index itself.
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
            match self.reindex_file(path) {
                Ok(n) => {
                    if n == 0 {
                        files_skipped += 1;
                    } else {
                        items_indexed += n;
                    }
                }
                Err(e) => errors.push((path.to_path_buf(), e.to_string())),
            }
        }

        // Lazy staleness sweep.
        let _ = self.store.sweep_stale();

        Ok(ReindexReport {
            files_scanned,
            items_indexed,
            files_skipped,
            errors,
        })
    }

    /// Reindex one file: delete its prior items, parse, upsert.
    /// Returns the number of items indexed (0 if the file is empty).
    pub fn reindex_file(&self, path: &Path) -> Result<usize> {
        let rel = relative_posix(path, &self.root);
        let items = html_parse::parse_file(path)?;
        // Replace strategy: simpler than diffing for MVP.
        self.store.delete_items_for_file(&rel)?;
        for item in &items {
            self.store.upsert_item(item, &rel, 0)?;
        }
        Ok(items.len())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReindexReport {
    pub files_scanned: usize,
    pub items_indexed: usize,
    pub files_skipped: usize,
    pub errors: Vec<(PathBuf, String)>,
}

fn relative_posix(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
