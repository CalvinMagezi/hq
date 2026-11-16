-- Shelving is how the owner tidies the task views. A shelved task or list is hidden only
-- from reads that ask for it to be (the web lists). The queue, the graph, dependencies,
-- leases, links, search and every agent tool ignore the column, so a shelved task stays
-- valid. It is separate from archived_at, which is a removal.
ALTER TABLE tasks ADD COLUMN shelved_at TEXT;
ALTER TABLE initiatives ADD COLUMN shelved_at TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_shelved ON tasks(shelved_at) WHERE shelved_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_initiatives_shelved ON initiatives(shelved_at) WHERE shelved_at IS NOT NULL;
