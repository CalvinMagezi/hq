-- One on-device vector per memory, so recall can find a memory by what it means when the
-- question shares no word with it ("which weekday do the feed trucks show up?" against "Acme
-- Feeds delivers on Thursdays"). The label of the model that made it is kept: a new model means
-- every row is made again. Deleting a memory deletes its vector with it.
CREATE TABLE IF NOT EXISTS memory_embeddings (
    memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    model     TEXT NOT NULL,
    vector    BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model ON memory_embeddings(model);
