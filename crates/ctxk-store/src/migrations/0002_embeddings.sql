-- Add embedding storage. Vectors stored as little-endian f32 BLOBs in
-- the items table itself; KNN happens in-process to avoid the
-- sqlite-vec loadable-extension portability pain.

ALTER TABLE items ADD COLUMN embedding BLOB;

-- Track the model that produced the embeddings so an upgrade triggers
-- a full re-embed rather than silently mixing dimensions.
INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '2');
