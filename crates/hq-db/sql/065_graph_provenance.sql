-- FR-063: every derived graph edge points at the note that supports it, and
-- every node remembers the page it was derived from. The tables stay a
-- rebuildable cache of `_graph/*.md`; nothing here is a source of truth.
ALTER TABLE entity_nodes ADD COLUMN page_path TEXT;
ALTER TABLE entity_nodes ADD COLUMN page_stamp TEXT;
ALTER TABLE entity_edges ADD COLUMN evidence_path TEXT;
ALTER TABLE entity_edges ADD COLUMN evidence_updated_at TEXT;
ALTER TABLE entity_edges ADD COLUMN confidence TEXT NOT NULL DEFAULT 'linked';

CREATE INDEX IF NOT EXISTS idx_edges_evidence ON entity_edges(evidence_path);
CREATE INDEX IF NOT EXISTS idx_entities_page ON entity_nodes(page_path);

-- One row. `degraded` means a rebuild or update failed and the index must not
-- be served as current until a rebuild or reconciliation succeeds.
CREATE TABLE IF NOT EXISTS entity_index_state (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    status     TEXT NOT NULL,
    error      TEXT,
    updated_at TEXT NOT NULL
);
