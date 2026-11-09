-- Sites the owner approved fetches from, per chat (crates/hq-db/src/fetch_sites.rs).
CREATE TABLE IF NOT EXISTS fetch_site_grants (
    host TEXT NOT NULL,
    scope TEXT NOT NULL,
    approvals INTEGER NOT NULL DEFAULT 0,
    whole_site INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (host, scope)
);
