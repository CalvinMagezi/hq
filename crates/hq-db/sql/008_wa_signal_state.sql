-- WhatsApp Signal Protocol State
-- Stores identity, sessions, and prekeys for E2E encryption.

CREATE TABLE IF NOT EXISTS wa_identities (
    device_id INTEGER NOT NULL,
    identity_key BLOB NOT NULL,
    registration_id INTEGER NOT NULL,
    record BLOB NOT NULL,
    PRIMARY KEY (device_id)
);

CREATE TABLE IF NOT EXISTS wa_sessions (
    device_id INTEGER NOT NULL,
    chat_jid TEXT NOT NULL,
    record BLOB NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (device_id, chat_jid)
);

CREATE TABLE IF NOT EXISTS wa_prekeys (
    device_id INTEGER NOT NULL,
    key_id INTEGER NOT NULL,
    record BLOB NOT NULL,
    PRIMARY KEY (device_id, key_id)
);

CREATE TABLE IF NOT EXISTS wa_signed_prekeys (
    device_id INTEGER NOT NULL,
    key_id INTEGER NOT NULL,
    record BLOB NOT NULL,
    PRIMARY KEY (device_id, key_id)
);

CREATE TABLE IF NOT EXISTS wa_sender_keys (
    device_id INTEGER NOT NULL,
    group_jid TEXT NOT NULL,
    sender_jid TEXT NOT NULL,
    record BLOB NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (device_id, group_jid, sender_jid)
);

CREATE TABLE IF NOT EXISTS wa_app_state_versions (
    device_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    hash BLOB,
    PRIMARY KEY (device_id, name)
);
