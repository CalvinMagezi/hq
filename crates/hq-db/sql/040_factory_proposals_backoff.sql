-- Exponential backoff on top of the existing verify_block_count ceiling: a
-- verification_blocked fingerprint used to be re-admittable on every scan
-- once it dropped out of `seen_fingerprints`, so a factory scanning every 6h
-- could retry the same broken pipeline dozens of times before hitting
-- MAX_VERIFY_BLOCK_ATTEMPTS. next_retry_at gates re-admission on wall-clock
-- time, not just attempt count: 1h after the 1st block, 4h after the 2nd,
-- 24h after the 3rd (which is also the give-up ceiling, so the row goes
-- straight to superseded and this column stops mattering for it).
ALTER TABLE factory_proposals ADD COLUMN next_retry_at TEXT;
