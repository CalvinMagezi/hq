-- Offers HQ showed under a reply (docs/plans/simple-hq.md, phase 2) and how the person answered.
-- Adoption data: it stays on the device, is never pushed and never reaches a model
-- (docs/security/ADOPTION_DATA.md). Only the offer's kind is kept, never the message.
CREATE TABLE IF NOT EXISTS offer_log (
    message_id TEXT PRIMARY KEY,
    offer TEXT NOT NULL,
    shown_at INTEGER NOT NULL,
    answer TEXT CHECK (answer IN ('yes', 'not_now')),
    answered_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_offer_log_offer ON offer_log(offer, shown_at);
