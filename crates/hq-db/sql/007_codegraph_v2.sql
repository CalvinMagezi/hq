-- Codegraph schema v2
-- Moves codegraph management to the central migration system.
-- We drop and recreate because this is a cache and old schemas are currently broken.

DROP TABLE IF EXISTS codegraph_edges;
DROP TABLE IF EXISTS codegraph_nodes;
DROP TABLE IF EXISTS codegraph_file_state;

CREATE TABLE codegraph_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    qualified_name TEXT NOT NULL,
    project_path TEXT NOT NULL DEFAULT '',
    file_path TEXT NOT NULL,
    name TEXT NOT NULL,
    node_type TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    signature TEXT,
    doc_comment TEXT,
    language TEXT NOT NULL
);

CREATE TABLE codegraph_edges (
    source_id INTEGER NOT NULL REFERENCES codegraph_nodes(id) ON DELETE CASCADE,
    target_id INTEGER NOT NULL REFERENCES codegraph_nodes(id) ON DELETE CASCADE,
    edge_type TEXT NOT NULL,
    PRIMARY KEY (source_id, target_id, edge_type)
);

CREATE TABLE codegraph_file_state (
    project_path TEXT NOT NULL DEFAULT '',
    file_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    node_count INTEGER NOT NULL DEFAULT 0,
    indexed_at INTEGER NOT NULL,
    PRIMARY KEY (project_path, file_path)
);

-- Indices
CREATE INDEX idx_cg_nodes_project ON codegraph_nodes(project_path);
CREATE INDEX idx_cg_nodes_file ON codegraph_nodes(file_path);
CREATE UNIQUE INDEX idx_cg_nodes_qname_project ON codegraph_nodes(project_path, qualified_name);
CREATE INDEX idx_cg_nodes_type ON codegraph_nodes(node_type);
CREATE INDEX idx_cg_edges_source ON codegraph_edges(source_id);
CREATE INDEX idx_cg_edges_target ON codegraph_edges(target_id);
