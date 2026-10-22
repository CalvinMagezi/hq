-- Native Bridge pairing (docs/security/PAIRING.md): the machines this one has paired with.
--
-- One row per peer, written only after the person at the machine showing the QR code typed the
-- right digits. A paired peer may connect to this machine's bridge service, and this machine may
-- dial it. Revoking sets revoked_at instead of deleting, so the history stays; pairing the same
-- key again clears it.
CREATE TABLE IF NOT EXISTS bridge_peers (
    -- The peer's iroh endpoint id, lowercase hex.
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL DEFAULT '',
    -- JSON arrays of the relay URLs and socket addresses the peer gave for dialing it back.
    relays TEXT NOT NULL DEFAULT '[]',
    addrs TEXT NOT NULL DEFAULT '[]',
    -- Unix seconds.
    paired_at INTEGER NOT NULL,
    last_seen_at INTEGER,
    revoked_at INTEGER
);
