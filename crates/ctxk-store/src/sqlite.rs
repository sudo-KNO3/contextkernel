//! SQLite handle, migrations, and CRUD for the items index.

use ctxk_core::{
    KnowledgeItem, KnowledgeType, Relation, Result, Scope, SourceType, Stability, Status,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

/// Thin wrapper around `rusqlite::Connection` with a process-wide mutex,
/// because SQLite is single-writer and we want predictable serialisation.
pub struct Store {
    inner: Mutex<Connection>,
    db_path: PathBuf,
}

impl Store {
    /// Open or create the SQLite index at `db_path`, applying migrations.
    pub fn open(db_path: impl Into<PathBuf>) -> Result<Self> {
        let db_path = db_path.into();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)
            .map_err(|e| ctxk_core::Error::Other(format!("opening {}: {e}", db_path.display())))?;
        for sql in MIGRATIONS {
            conn.execute_batch(sql)
                .map_err(|e| ctxk_core::Error::Other(format!("migration: {e}")))?;
        }
        Ok(Self {
            inner: Mutex::new(conn),
            db_path,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Upsert one item. Deletes prior tags/relations for the id first so the
    /// state is always a clean reflection of what's currently in the HTML.
    pub fn upsert_item(&self, item: &KnowledgeItem, file_path: &str, file_offset: i64) -> Result<()> {
        let conn = self.inner.lock().unwrap();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| ctxk_core::Error::Other(format!("begin tx: {e}")))?;

        // 1. items row (upsert)
        let tags_concat = item.tags.join(" ");
        let relations_json = serde_json::to_string(&item.relations)
            .map_err(|e| ctxk_core::Error::Other(format!("serialize relations: {e}")))?;
        let claim_key = item.derived_claim_key();
        tx.execute(
            "INSERT INTO items (
                id, file_path, file_offset, knowledge_type, scope, confidence,
                source_type, status, stability, created, modified,
                valid_from, valid_until, domain, title, body_text, body_html,
                tags_concat, relations_json, claim_key, content_hash
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
            )
            ON CONFLICT(id) DO UPDATE SET
                file_path=excluded.file_path,
                file_offset=excluded.file_offset,
                knowledge_type=excluded.knowledge_type,
                scope=excluded.scope,
                confidence=excluded.confidence,
                source_type=excluded.source_type,
                status=excluded.status,
                stability=excluded.stability,
                created=excluded.created,
                modified=excluded.modified,
                valid_from=excluded.valid_from,
                valid_until=excluded.valid_until,
                domain=excluded.domain,
                title=excluded.title,
                body_text=excluded.body_text,
                body_html=excluded.body_html,
                tags_concat=excluded.tags_concat,
                relations_json=excluded.relations_json,
                claim_key=excluded.claim_key,
                content_hash=excluded.content_hash",
            params![
                item.id,
                file_path,
                file_offset,
                item.knowledge_type.as_str(),
                item.scope.as_str(),
                item.confidence,
                item.source_type.as_str(),
                item.status.as_str(),
                item.stability.as_str(),
                item.created.format(&Rfc3339).unwrap_or_default(),
                item.modified.format(&Rfc3339).unwrap_or_default(),
                item.valid_from.as_ref().and_then(|d| d.format(&Rfc3339).ok()),
                item.valid_until.as_ref().and_then(|d| d.format(&Rfc3339).ok()),
                item.domain,
                item.title,
                item.body_text,
                item.body_html,
                tags_concat,
                relations_json,
                claim_key,
                content_hash(&item.body_text),
            ],
        )
        .map_err(|e| ctxk_core::Error::Other(format!("upsert items: {e}")))?;

        // 2. tags rows (replace)
        tx.execute("DELETE FROM tags WHERE item_id = ?1", params![item.id])
            .map_err(|e| ctxk_core::Error::Other(format!("clear tags: {e}")))?;
        for tag in &item.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags(item_id, tag) VALUES (?1, ?2)",
                params![item.id, tag],
            )
            .map_err(|e| ctxk_core::Error::Other(format!("insert tag: {e}")))?;
        }

        // 3. relations rows (replace)
        tx.execute("DELETE FROM relations WHERE src_id = ?1", params![item.id])
            .map_err(|e| ctxk_core::Error::Other(format!("clear relations: {e}")))?;
        for rel in &item.relations {
            tx.execute(
                "INSERT OR IGNORE INTO relations(src_id, rel, dst_id) VALUES (?1, ?2, ?3)",
                params![item.id, rel.rel, rel.target],
            )
            .map_err(|e| ctxk_core::Error::Other(format!("insert relation: {e}")))?;
        }

        // 4. FTS sync — self-contained FTS5 table, joined by id.
        tx.execute("DELETE FROM items_fts WHERE id = ?1", params![item.id])
            .ok();
        tx.execute(
            "INSERT INTO items_fts(id, title, body_text, tags_concat)
             VALUES (?1, ?2, ?3, ?4)",
            params![item.id, item.title, item.body_text, tags_concat],
        )
        .map_err(|e| ctxk_core::Error::Other(format!("fts insert: {e}")))?;

        tx.commit()
            .map_err(|e| ctxk_core::Error::Other(format!("commit: {e}")))?;
        Ok(())
    }

    /// Drop all items belonging to `file_path` — used during reindex when a
    /// file's contents have changed.
    pub fn delete_items_for_file(&self, file_path: &str) -> Result<usize> {
        let conn = self.inner.lock().unwrap();
        // Cascade does the rest (tags/relations); FTS doesn't cascade so
        // manually purge.
        let to_delete: Vec<String> = conn
            .prepare("SELECT id FROM items WHERE file_path = ?1")
            .and_then(|mut stmt| {
                let rows = stmt.query_map(params![file_path], |r| r.get::<_, String>(0))?;
                rows.collect()
            })
            .map_err(|e| ctxk_core::Error::Other(format!("select-for-delete: {e}")))?;
        for id in &to_delete {
            conn.execute("DELETE FROM items_fts WHERE id = ?1", params![id]).ok();
        }
        let n = conn
            .execute("DELETE FROM items WHERE file_path = ?1", params![file_path])
            .map_err(|e| ctxk_core::Error::Other(format!("delete file items: {e}")))?;
        Ok(n)
    }

    /// Fetch one item by id, or None.
    pub fn get_item(&self, id: &str) -> Result<Option<KnowledgeItem>> {
        let conn = self.inner.lock().unwrap();
        conn.query_row(
            "SELECT id, knowledge_type, scope, confidence, source_type, status, stability,
                    created, modified, valid_from, valid_until, domain, title, body_text,
                    body_html, tags_concat, relations_json, claim_key
             FROM items WHERE id = ?1",
            params![id],
            row_to_item,
        )
        .optional()
        .map_err(|e| ctxk_core::Error::Other(format!("get_item: {e}")))
    }

    /// Free-form list with optional filters. `q` is FTS5 MATCH; the rest are
    /// equality filters.
    pub fn list_items(&self, f: &ListFilters) -> Result<Vec<KnowledgeItem>> {
        let conn = self.inner.lock().unwrap();
        let (sql, args) = build_list_query(f);
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ctxk_core::Error::Other(format!("prepare list: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), row_to_item)
            .map_err(|e| ctxk_core::Error::Other(format!("query list: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| ctxk_core::Error::Other(format!("collect list: {e}")))
    }

    /// Get just (id, content_hash, modified) for every item in a file, used
    /// by reindex to skip unchanged sections.
    pub fn file_index(&self, file_path: &str) -> Result<Vec<(String, String)>> {
        let conn = self.inner.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, content_hash FROM items WHERE file_path = ?1")
            .map_err(|e| ctxk_core::Error::Other(format!("prepare file_index: {e}")))?;
        let rows = stmt
            .query_map(params![file_path], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| ctxk_core::Error::Other(format!("query file_index: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| ctxk_core::Error::Other(format!("collect file_index: {e}")))
    }

    /// Summary stats for `GET /vault/stats`.
    pub fn stats(&self) -> Result<VaultStats> {
        let conn = self.inner.lock().unwrap();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .unwrap_or(0);
        let mut by_scope = Vec::new();
        let mut stmt = conn
            .prepare("SELECT scope, COUNT(*) FROM items GROUP BY scope")
            .map_err(|e| ctxk_core::Error::Other(format!("stats scope: {e}")))?;
        for row in stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| ctxk_core::Error::Other(format!("stats scope iter: {e}")))?
        {
            if let Ok(p) = row {
                by_scope.push(p);
            }
        }
        let mut by_type = Vec::new();
        let mut stmt = conn
            .prepare("SELECT knowledge_type, COUNT(*) FROM items GROUP BY knowledge_type")
            .map_err(|e| ctxk_core::Error::Other(format!("stats type: {e}")))?;
        for row in stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| ctxk_core::Error::Other(format!("stats type iter: {e}")))?
        {
            if let Ok(p) = row {
                by_type.push(p);
            }
        }
        Ok(VaultStats {
            total,
            by_scope,
            by_type,
        })
    }

    /// Mark items past `valid_until` as stale (lazy sweep called before queries).
    pub fn sweep_stale(&self) -> Result<usize> {
        let conn = self.inner.lock().unwrap();
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default();
        let n = conn
            .execute(
                "UPDATE items
                 SET status = 'stale'
                 WHERE status = 'active'
                   AND valid_until IS NOT NULL
                   AND valid_until != ''
                   AND valid_until < ?1",
                params![now],
            )
            .map_err(|e| ctxk_core::Error::Other(format!("sweep_stale: {e}")))?;
        Ok(n)
    }

    /// Append a row to the review queue.
    pub fn queue_propose(&self, kind: &str, target_id: Option<&str>, proposed_by: &str,
                          payload_json: &str, rationale: Option<&str>) -> Result<String> {
        let conn = self.inner.lock().unwrap();
        let queue_id = ctxk_core::new_id();
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default();
        conn.execute(
            "INSERT INTO review_queue (id, kind, target_id, proposed_by, proposed_at, status, payload_json, rationale)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
            params![queue_id, kind, target_id, proposed_by, now, payload_json, rationale],
        )
        .map_err(|e| ctxk_core::Error::Other(format!("queue_propose: {e}")))?;
        Ok(queue_id)
    }

    pub fn queue_list(&self, status: Option<&str>) -> Result<Vec<QueueEntry>> {
        let conn = self.inner.lock().unwrap();
        let (sql, status_filter) = match status {
            Some(s) => (
                "SELECT id, kind, target_id, proposed_by, proposed_at, status, payload_json, rationale
                 FROM review_queue WHERE status = ?1 ORDER BY proposed_at DESC",
                Some(s.to_string()),
            ),
            None => (
                "SELECT id, kind, target_id, proposed_by, proposed_at, status, payload_json, rationale
                 FROM review_queue ORDER BY proposed_at DESC",
                None,
            ),
        };
        let mut stmt = conn.prepare(sql).map_err(|e| ctxk_core::Error::Other(format!("prep queue_list: {e}")))?;
        let mapper = |r: &rusqlite::Row| -> rusqlite::Result<QueueEntry> {
            Ok(QueueEntry {
                id: r.get(0)?,
                kind: r.get(1)?,
                target_id: r.get(2)?,
                proposed_by: r.get(3)?,
                proposed_at: r.get(4)?,
                status: r.get(5)?,
                payload_json: r.get(6)?,
                rationale: r.get(7)?,
            })
        };
        let rows: Vec<QueueEntry> = match status_filter {
            Some(s) => stmt
                .query_map(params![s], mapper)
                .map_err(|e| ctxk_core::Error::Other(format!("queue_list query: {e}")))?
                .filter_map(|r| r.ok())
                .collect(),
            None => stmt
                .query_map([], mapper)
                .map_err(|e| ctxk_core::Error::Other(format!("queue_list query: {e}")))?
                .filter_map(|r| r.ok())
                .collect(),
        };
        Ok(rows)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ListFilters {
    pub scope: Option<String>,
    pub knowledge_type: Option<String>,
    pub domain: Option<String>,
    pub tag: Option<String>,
    pub q: Option<String>, // FTS5 MATCH
    pub include_status: Vec<String>, // empty => exclude 'deleted'
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VaultStats {
    pub total: i64,
    pub by_scope: Vec<(String, i64)>,
    pub by_type: Vec<(String, i64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueEntry {
    pub id: String,
    pub kind: String,
    pub target_id: Option<String>,
    pub proposed_by: String,
    pub proposed_at: String,
    pub status: String,
    pub payload_json: String,
    pub rationale: Option<String>,
}

fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    if s.is_empty() {
        None
    } else {
        OffsetDateTime::parse(s, &Rfc3339).ok()
    }
}

fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeItem> {
    let id: String = row.get(0)?;
    let ktype: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let confidence: f64 = row.get(3)?;
    let source_type: String = row.get(4)?;
    let status: String = row.get(5)?;
    let stability: String = row.get(6)?;
    let created: String = row.get(7)?;
    let modified: String = row.get(8)?;
    let valid_from: Option<String> = row.get(9)?;
    let valid_until: Option<String> = row.get(10)?;
    let domain: Option<String> = row.get(11)?;
    let title: String = row.get(12)?;
    let body_text: String = row.get(13)?;
    let body_html: String = row.get(14)?;
    let tags_concat: String = row.get(15)?;
    let relations_json: String = row.get(16)?;
    let claim_key: Option<String> = row.get(17)?;

    let tags = if tags_concat.trim().is_empty() {
        Vec::new()
    } else {
        tags_concat.split_whitespace().map(|s| s.to_string()).collect()
    };
    let relations: Vec<Relation> = serde_json::from_str(&relations_json).unwrap_or_default();

    Ok(KnowledgeItem {
        id,
        knowledge_type: KnowledgeType::parse(&ktype),
        scope: Scope::parse(&scope),
        confidence,
        source_type: SourceType::parse(&source_type),
        status: Status::parse(&status),
        stability: Stability::parse(&stability),
        created: parse_rfc3339(&created).unwrap_or_else(OffsetDateTime::now_utc),
        modified: parse_rfc3339(&modified).unwrap_or_else(OffsetDateTime::now_utc),
        valid_from: valid_from.as_deref().and_then(parse_rfc3339),
        valid_until: valid_until.as_deref().and_then(parse_rfc3339),
        domain,
        tags,
        title,
        body_text,
        body_html,
        relations,
        claim_key,
    })
}

fn build_list_query(f: &ListFilters) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut sql = String::from(
        "SELECT i.id, i.knowledge_type, i.scope, i.confidence, i.source_type, i.status, i.stability,
                i.created, i.modified, i.valid_from, i.valid_until, i.domain, i.title, i.body_text,
                i.body_html, i.tags_concat, i.relations_json, i.claim_key
         FROM items i",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut wheres: Vec<String> = Vec::new();

    if let Some(q) = &f.q {
        sql.push_str(" JOIN items_fts ON items_fts.id = i.id ");
        wheres.push("items_fts MATCH ?".to_string());
        args.push(Box::new(q.clone()));
    }
    if let Some(s) = &f.scope {
        wheres.push("i.scope = ?".into());
        args.push(Box::new(s.clone()));
    }
    if let Some(t) = &f.knowledge_type {
        wheres.push("i.knowledge_type = ?".into());
        args.push(Box::new(t.clone()));
    }
    if let Some(d) = &f.domain {
        wheres.push("i.domain = ?".into());
        args.push(Box::new(d.clone()));
    }
    if let Some(tag) = &f.tag {
        wheres.push("EXISTS (SELECT 1 FROM tags WHERE tags.item_id = i.id AND tags.tag = ?)".into());
        args.push(Box::new(tag.clone()));
    }
    if f.include_status.is_empty() {
        wheres.push("i.status != 'deleted'".into());
    } else {
        let placeholders = vec!["?"; f.include_status.len()].join(", ");
        wheres.push(format!("i.status IN ({})", placeholders));
        for s in &f.include_status {
            args.push(Box::new(s.clone()));
        }
    }

    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&wheres.join(" AND "));
    }
    if f.q.is_some() {
        sql.push_str(" ORDER BY rank");
    } else {
        sql.push_str(" ORDER BY i.modified DESC");
    }
    if let Some(limit) = f.limit {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    if let Some(offset) = f.offset {
        sql.push_str(&format!(" OFFSET {}", offset));
    }

    (sql, args)
}
