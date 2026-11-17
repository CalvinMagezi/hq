-- Folders the owner allowed for one chat, so that chat's file edits and shell commands inside
-- them run without an approval each (docs/security/OWNER_APPROVAL.md, "Folder allow").
-- Written only by the owner's own answer route, never by a tool a model can call. A grant
-- lapses at expires_at (eight hours by default) and the owner can revoke it at any time.
CREATE TABLE IF NOT EXISTS owner_folder_grants (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The chat the grant belongs to, as the approval keys name it (`web:<thread>`).
    scope        TEXT NOT NULL,
    -- Canonical absolute path of the folder.
    root         TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    revoked_at   INTEGER,
    -- Calls this grant let through without asking, and when the last one ran.
    uses         INTEGER NOT NULL DEFAULT 0,
    last_used_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS owner_folder_grants_live
    ON owner_folder_grants (scope, root) WHERE revoked_at IS NULL;
