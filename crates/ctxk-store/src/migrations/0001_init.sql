-- ContextKernel SQLite schema, v1.
--
-- HTML files in the vault are the source of truth. These tables are a
-- derived index that can be dropped and rebuilt with `Vault::reindex_all()`.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS items (
    id              TEXT PRIMARY KEY,        -- ULID
    file_path       TEXT NOT NULL,           -- POSIX-style, relative to vault root
    file_offset     INTEGER NOT NULL DEFAULT 0,
    knowledge_type  TEXT NOT NULL,
    scope           TEXT NOT NULL,
    confidence      REAL NOT NULL DEFAULT 0.5,
    source_type     TEXT NOT NULL DEFAULT 'user',
    status          TEXT NOT NULL DEFAULT 'active',
    stability       TEXT NOT NULL DEFAULT 'medium-term',
    created         TEXT NOT NULL,           -- RFC3339
    modified        TEXT NOT NULL,
    valid_from      TEXT,
    valid_until     TEXT,
    domain          TEXT,
    title           TEXT NOT NULL,
    body_text       TEXT NOT NULL,
    body_html       TEXT NOT NULL,
    tags_concat     TEXT NOT NULL DEFAULT '',  -- space-separated, mirrored for FTS
    relations_json  TEXT NOT NULL DEFAULT '[]',
    claim_key       TEXT,
    content_hash    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_items_scope_type  ON items(scope, knowledge_type);
CREATE INDEX IF NOT EXISTS idx_items_domain      ON items(domain);
CREATE INDEX IF NOT EXISTS idx_items_status      ON items(status);
CREATE INDEX IF NOT EXISTS idx_items_valid_until ON items(valid_until);
CREATE INDEX IF NOT EXISTS idx_items_claim_key   ON items(claim_key);

CREATE TABLE IF NOT EXISTS tags (
    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    tag     TEXT NOT NULL,
    PRIMARY KEY (item_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);

CREATE TABLE IF NOT EXISTS relations (
    src_id   TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    rel      TEXT NOT NULL,
    dst_id   TEXT NOT NULL,
    PRIMARY KEY (src_id, rel, dst_id)
);
CREATE INDEX IF NOT EXISTS idx_rel_dst ON relations(dst_id, rel);

-- FTS5 index over title + body + tag concat. Self-contained (FTS5 keeps
-- its own copy) so delete/insert are trivial and we don't pay the
-- external-content gotchas. Joined back to items via the unindexed `id`.
CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
    id UNINDEXED,
    title, body_text, tags_concat,
    tokenize='porter unicode61'
);

CREATE TABLE IF NOT EXISTS review_queue (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,       -- 'new' | 'update'
    target_id     TEXT,                -- for kind='update'
    proposed_by   TEXT NOT NULL,
    proposed_at   TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending',
    payload_json  TEXT NOT NULL,
    rationale     TEXT,
    reviewed_at   TEXT,
    reviewed_by   TEXT,
    decision_note TEXT
);
CREATE INDEX IF NOT EXISTS idx_queue_status ON review_queue(status, proposed_at);

CREATE TABLE IF NOT EXISTS schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('schema_version', '1');
