ALTER TABLE proxy_calls ADD COLUMN executor TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE proxy_calls ADD COLUMN spend_cap_usd REAL;
ALTER TABLE proxy_calls ADD COLUMN rejected_reason TEXT;
