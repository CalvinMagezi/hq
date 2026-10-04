-- Owner approvals are signed with a per-install key that lives outside the database, and a
-- self-update approval is bound to the SHA-256 of the built binary.
ALTER TABLE value_items ADD COLUMN engagement_mac TEXT;
ALTER TABLE self_update_runs ADD COLUMN approved_binary_sha256 TEXT;
ALTER TABLE self_update_runs ADD COLUMN approval_mac TEXT;
