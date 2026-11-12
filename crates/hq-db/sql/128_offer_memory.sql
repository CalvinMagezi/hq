-- "Later" and a remembered "Not now" for offers (docs/plans/offers-later.md).
-- Adoption data: it stays on the device, is never pushed and never reaches a model
-- (docs/security/ADOPTION_DATA.md). Only an offer's type is kept, never a message.

-- offer_log gains the answer 'later'. SQLite cannot widen a CHECK, so the table is rebuilt.
DROP TABLE IF EXISTS offer_log_new;
CREATE TABLE offer_log_new (
    message_id TEXT PRIMARY KEY,
    offer TEXT NOT NULL,
    shown_at INTEGER NOT NULL,
    answer TEXT CHECK (answer IN ('yes', 'not_now', 'later')),
    answered_at INTEGER,
    -- Stamped by a Clear: the row stops counting in Settings and Not now memory, but keeps
    -- feeding the pacing (hourly cap, days between offers).
    cleared_at INTEGER,
    -- 1 for an offer that came back (Later, scheduled): it must not block fresh offers.
    resurfaced INTEGER NOT NULL DEFAULT 0
);
INSERT INTO offer_log_new (message_id, offer, shown_at, answer, answered_at)
    SELECT message_id, offer, shown_at, answer, answered_at FROM offer_log;
DROP TABLE offer_log;
ALTER TABLE offer_log_new RENAME TO offer_log;
CREATE INDEX IF NOT EXISTS idx_offer_log_offer ON offer_log(offer, shown_at);

-- What the person told HQ about one TYPE of offer (an offer kind or a capability id), not an item.
CREATE TABLE IF NOT EXISTS offer_type_memory (
    offer TEXT PRIMARY KEY,
    -- Not nows in a row; a Yes or a pause resets it.
    not_now_run INTEGER NOT NULL DEFAULT 0,
    -- The type stays out of the chooser until this unix second.
    hidden_until INTEGER NOT NULL DEFAULT 0,
    -- "Never show": 1 until the person clears their choices.
    never INTEGER NOT NULL DEFAULT 0,
    never_at INTEGER NOT NULL DEFAULT 0,
    -- "Fewer like this": until this unix second a Not now hides the type twice as long.
    fewer_until INTEGER NOT NULL DEFAULT 0
);

-- A Later offer waiting for its one resurfacing. The inbox item is the resurfacing; the row goes
-- when the person answers that item (the offer returns to the chat) or the item is gone.
CREATE TABLE IF NOT EXISTS offer_later (
    item_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    offer TEXT NOT NULL,
    line TEXT NOT NULL,
    reply TEXT NOT NULL,
    send INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_offer_later_message ON offer_later(message_id);
