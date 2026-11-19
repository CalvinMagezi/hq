-- Questions HQ puts to the owner with a default and a deadline (crates/hq-db/src/owner_decisions.rs).
CREATE TABLE IF NOT EXISTS owner_decisions (
    id TEXT PRIMARY KEY,
    question TEXT NOT NULL,
    options TEXT NOT NULL,
    default_option TEXT NOT NULL,
    deadline_at TEXT NOT NULL,
    reversible INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','answered','defaulted','held')),
    chosen TEXT,
    item_id TEXT,
    created_at TEXT NOT NULL,
    resolved_at TEXT
);
