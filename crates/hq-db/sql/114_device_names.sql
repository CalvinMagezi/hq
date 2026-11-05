-- Device sign-ins by name and with an idle expiry (docs/plans/ux-run/web/wave-1/05-sign-in-safety.md).
-- `idle_expires_after` (seconds) is set on a code sign-in: unused that long, it stops working unless
-- `keep` is set. An expired row stays, refused and hidden, so signing in stays on (`sign_in_enabled`).
ALTER TABLE web_device_tokens ADD COLUMN browser TEXT;
ALTER TABLE web_device_tokens ADD COLUMN system TEXT;
ALTER TABLE web_device_tokens ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
ALTER TABLE web_device_tokens ADD COLUMN keep INTEGER NOT NULL DEFAULT 0;
ALTER TABLE web_device_tokens ADD COLUMN idle_expires_after INTEGER;
