ALTER TABLE background_turns ADD COLUMN kind TEXT NOT NULL DEFAULT 'turn';
ALTER TABLE background_turns ADD COLUMN watch_interval_secs INTEGER;
ALTER TABLE background_turns ADD COLUMN watch_until INTEGER; -- unix epoch, NULL = no expiry
ALTER TABLE background_turns ADD COLUMN watch_last_fired INTEGER; -- unix epoch
