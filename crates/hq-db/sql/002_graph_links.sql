-- Graph links between notes (semantic similarity, explicit references, etc.)
CREATE TABLE IF NOT EXISTS graph_links (
    source_path TEXT NOT NULL,
    target_path TEXT NOT NULL,
    score REAL NOT NULL,
    link_type TEXT NOT NULL DEFAULT 'semantic',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (source_path, target_path)
);

-- Link state tracking (change detection for incremental re-linking)
CREATE TABLE IF NOT EXISTS link_state (
    note_path TEXT PRIMARY KEY,
    last_linked_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);

-- Indexes for graph traversal
CREATE INDEX IF NOT EXISTS idx_graph_links_source ON graph_links(source_path);
CREATE INDEX IF NOT EXISTS idx_graph_links_target ON graph_links(target_path);
CREATE INDEX IF NOT EXISTS idx_embeddings_path ON embeddings(note_path);
