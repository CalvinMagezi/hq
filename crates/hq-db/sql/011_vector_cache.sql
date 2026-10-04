-- Vector Cache Table: Native vector storage using sqlite-vec
-- Compatible with 384-dimension vectors from all-MiniLM-L6-v2
CREATE TABLE IF NOT EXISTS vault_embeddings (
    path TEXT PRIMARY KEY,
    embedding F32_BLOB(384) NOT NULL,
    model_name TEXT NOT NULL,
    FOREIGN KEY(path) REFERENCES vault_cache(path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_vault_embeddings_path ON vault_embeddings(path);
