-- What each browser's push subscription receives (docs/WEB_PUSH.md). `kinds` is a comma list of
-- `needs` and `updates`; `details` lets a push carry an item's first line, off by default.
ALTER TABLE push_subscriptions ADD COLUMN kinds TEXT NOT NULL DEFAULT 'needs,updates';
ALTER TABLE push_subscriptions ADD COLUMN details INTEGER NOT NULL DEFAULT 0;
