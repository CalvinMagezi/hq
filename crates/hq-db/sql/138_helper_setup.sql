-- What HQ knows about the coding helpers on each computer (docs/HELPER_SETUP.md): whether the
-- program is there and whether it was signed in when last checked. A reading, not a promise:
-- `checked_at` says how old it is. signed_in is NULL when HQ has no way to ask the program.
CREATE TABLE IF NOT EXISTS helper_status (
    host        TEXT NOT NULL,
    helper      TEXT NOT NULL,
    installed   INTEGER NOT NULL,
    signed_in   INTEGER,
    checked_at  INTEGER NOT NULL,
    last_error  TEXT,
    PRIMARY KEY (host, helper)
);

-- One install, sign-in or check HQ ran on a computer, so the apps can show how it is going and
-- what the program printed (a sign-in link, for a computer with no browser).
CREATE TABLE IF NOT EXISTS helper_setup_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    host        TEXT NOT NULL,
    helper      TEXT NOT NULL,
    -- install | login | check
    step        TEXT NOT NULL,
    -- running | ok | failed
    state       TEXT NOT NULL,
    -- The last lines the program printed, with secrets-shaped text removed.
    tail        TEXT NOT NULL DEFAULT '',
    -- Links the program printed (JSON array of strings).
    urls        TEXT NOT NULL DEFAULT '[]',
    started_at  INTEGER NOT NULL,
    finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS helper_setup_runs_recent ON helper_setup_runs (host, helper, id DESC);
