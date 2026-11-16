-- Domains the owner trusts for fetches, set by hand (crates/hq-db/src/fetch_sites.rs).
CREATE TABLE IF NOT EXISTS fetch_trusted_domains (
    domain TEXT PRIMARY KEY,
    added_at TEXT NOT NULL
);
