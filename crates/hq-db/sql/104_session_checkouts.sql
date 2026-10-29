-- Where a coding session writes (docs/plans/remote-driven-teams.md, phase 4): its own git
-- worktree, or the paths it owns in a folder it shares, and the last commit seen there.
CREATE TABLE IF NOT EXISTS session_checkouts (
    session_id TEXT PRIMARY KEY,
    -- The folder the session was asked to start in.
    folder TEXT NOT NULL,
    -- Its own worktree, under the host's HQ folder, when it has one.
    worktree TEXT,
    -- The repository's main folder, for a worktree.
    repo TEXT,
    -- A writer's branch, or a reviewer's commit (one of the two, for a worktree).
    branch TEXT,
    detached TEXT,
    -- JSON list of paths it owns in a shared folder; [] is the whole folder.
    owns TEXT NOT NULL DEFAULT '[]',
    -- The last commit seen at a finished turn, so a new one becomes a `commit` event once.
    last_head TEXT,
    -- When its worktree was removed (a stop or an exit with nothing uncommitted).
    removed_at TEXT
);
