-- Code-aware items: track the source file + line range a knowledge item
-- was extracted from (when applicable). Powers folder_proximity scoring
-- and anchor_path resolution at retrieval time.

ALTER TABLE items ADD COLUMN defined_path TEXT;
ALTER TABLE items ADD COLUMN defined_start_line INTEGER;
ALTER TABLE items ADD COLUMN defined_end_line INTEGER;

CREATE INDEX IF NOT EXISTS idx_items_defined_path ON items(defined_path);

INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '3');
