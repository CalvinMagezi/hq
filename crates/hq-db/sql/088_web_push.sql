-- Web Push (docs/WEB_PUSH.md). An inbox item can open a page of the web app, and records
-- when the push dispatcher took it, so two server processes never push it twice.
ALTER TABLE value_items ADD COLUMN link TEXT;
ALTER TABLE value_items ADD COLUMN pushed_at TEXT;
-- Everything already in the inbox is history: it is never pushed.
UPDATE value_items SET pushed_at = created_at;
CREATE INDEX IF NOT EXISTS idx_value_items_unpushed ON value_items(created_at) WHERE pushed_at IS NULL;

-- One row per browser that turned push on. The keys come from its PushSubscription.
CREATE TABLE IF NOT EXISTS push_subscriptions (
    endpoint     TEXT PRIMARY KEY,
    p256dh       TEXT NOT NULL,
    auth         TEXT NOT NULL,
    user_agent   TEXT,
    created_at   TEXT NOT NULL,
    last_ok_at   TEXT,
    failures     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT
);
