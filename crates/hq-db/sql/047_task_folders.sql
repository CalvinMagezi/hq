-- Folders as a first-class level between Space and Initiative, matching
-- ClickUp's own Space > Folder > List hierarchy (the migration's initial cut
-- flattened "Folder / List" into a single initiative name string; this
-- makes Folder a real, queryable relationship instead).

CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES spaces(id),
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(space_id, slug)
);

-- NULL = folderless, directly under the space (ClickUp allows both).
ALTER TABLE initiatives ADD COLUMN folder_id TEXT REFERENCES folders(id);

CREATE INDEX IF NOT EXISTS idx_initiatives_folder ON initiatives(folder_id);
