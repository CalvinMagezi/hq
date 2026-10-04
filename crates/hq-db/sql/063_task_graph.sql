-- Derived task relationship index, logically separate from the vault/memory
-- graph. Rebuildable from tasks/task_tags at any time; nothing else reads it.
-- No foreign keys on purpose: deleting a task must never be blocked by, or
-- depend on, this index. Orphans are pruned by task_graph::sync.
CREATE TABLE IF NOT EXISTS task_graph_nodes (
    task_id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
-- One row per unordered pair (task_a < task_b). Inferred similarity only;
-- explicit links (parent, depends_on) are read live from the source tables.
CREATE TABLE IF NOT EXISTS task_graph_edges (
    task_a TEXT NOT NULL,
    task_b TEXT NOT NULL,
    score REAL NOT NULL,
    evidence TEXT NOT NULL,
    PRIMARY KEY (task_a, task_b),
    CHECK (task_a < task_b)
);
CREATE INDEX IF NOT EXISTS idx_task_graph_edges_b ON task_graph_edges(task_b, score);
