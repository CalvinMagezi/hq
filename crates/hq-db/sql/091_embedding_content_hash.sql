-- The hash of the note (sync_state.content_hash) an embedding was made from, so an edited note
-- can be told apart from an unchanged one and embedded again. NULL for embeddings made before
-- this column existed; those are judged stale only if the note changed after they were made.
ALTER TABLE embeddings ADD COLUMN content_hash TEXT;
