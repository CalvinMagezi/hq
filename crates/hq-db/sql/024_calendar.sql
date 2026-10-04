CREATE TABLE IF NOT EXISTS calendar_accounts (
    account_id    TEXT PRIMARY KEY,
    provider      TEXT NOT NULL,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    color         TEXT,
    sync_token    TEXT,
    last_sync_at  INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS calendars (
    calendar_id   TEXT NOT NULL,
    account_id    TEXT NOT NULL REFERENCES calendar_accounts(account_id) ON DELETE CASCADE,
    name          TEXT,
    description   TEXT,
    color         TEXT,
    is_primary    INTEGER NOT NULL DEFAULT 0,
    selected      INTEGER NOT NULL DEFAULT 1,
    sync_token    TEXT,
    PRIMARY KEY (account_id, calendar_id)
);

CREATE TABLE IF NOT EXISTS calendar_events (
    event_id        TEXT NOT NULL,
    account_id      TEXT NOT NULL REFERENCES calendar_accounts(account_id) ON DELETE CASCADE,
    calendar_id     TEXT NOT NULL,
    ical_uid        TEXT,
    etag            TEXT,
    title           TEXT,
    description     TEXT,
    location        TEXT,
    start_ts        INTEGER NOT NULL,
    end_ts          INTEGER NOT NULL,
    all_day         INTEGER NOT NULL DEFAULT 0,
    status          TEXT,
    organizer       TEXT,
    attendees_json  TEXT,
    recurrence_json TEXT,
    html_link       TEXT,
    updated_at      INTEGER NOT NULL,
    deleted         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, calendar_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_cal_events_range ON calendar_events(start_ts, end_ts);
CREATE INDEX IF NOT EXISTS idx_cal_events_account ON calendar_events(account_id);
