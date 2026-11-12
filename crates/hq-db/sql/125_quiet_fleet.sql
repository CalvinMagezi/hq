-- Quiet fleet (docs/plans/quiet-fleet.md). `owner_only` marks a request only the owner can
-- decide; the runtime that raises the item sets it, no tool argument can. `expired_reason`
-- and `expired_at` record why an item left Needs you by itself (the daily tidy), and
-- `restored_at` keeps the tidy from taking a restored item a second time.
ALTER TABLE value_items ADD COLUMN owner_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE value_items ADD COLUMN expired_reason TEXT;
ALTER TABLE value_items ADD COLUMN expired_at TEXT;
ALTER TABLE value_items ADD COLUMN restored_at TEXT;

-- Backfill by rule: sign-ins, connector and chat approvals, and proposals are the owner's.
-- Helper prompts and finished notices stay false, so the first tidy can clear the old pile.
UPDATE value_items SET owner_only = 1
 WHERE source_task IN ('device-sign-in', 'mcp-connector', 'owner-approval')
    OR kind = 'proposal'
    OR id IN (SELECT item_id FROM owner_approvals);

-- Before this migration nothing recorded which session prompts would lose work, but a prompt
-- its command policy escalated says so in its body. Those stay owner-only; every other old helper
-- prompt is helper noise, so the first tidy can clear it once it is stale or its session is gone.
UPDATE value_items SET owner_only = 1
 WHERE source_task IN ('session-supervisor', 'session_chat')
   AND dedup_key LIKE 'session-blocked-%'
   AND (body LIKE '%command policy left this to you%' OR body LIKE '%command policy could not settle this%');

CREATE INDEX IF NOT EXISTS idx_value_items_expired ON value_items(expired_at) WHERE expired_at IS NOT NULL;
