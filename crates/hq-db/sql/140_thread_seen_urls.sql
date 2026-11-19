-- URLs HQ itself saw arrive from outside in a chat thread (search results, fetched pages, the
-- owner's own words). A tainted web_fetch of exactly one of them needs no approval.
CREATE TABLE IF NOT EXISTS thread_seen_urls (
    thread_id TEXT NOT NULL,
    url TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, url)
);
CREATE INDEX IF NOT EXISTS idx_thread_seen_urls_seen_at ON thread_seen_urls (seen_at);
