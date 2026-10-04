CREATE TABLE IF NOT EXISTS company_spend (
    id          INTEGER PRIMARY KEY,
    company_id  TEXT    NOT NULL,
    month       TEXT    NOT NULL,
    spend_usd   REAL    NOT NULL DEFAULT 0,
    token_count INTEGER NOT NULL DEFAULT 0,
    recorded_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_company_spend_lookup
    ON company_spend (company_id, month);
