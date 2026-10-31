-- What pairing set up with each peer (docs/plans/pairing-autoconfig.md, `hq/setup/1`). A table of
-- its own rather than a JSON column on bridge_peers: revoke deletes the row whole, and the readers
-- (the forwards, the host list, task_peer) select by role without parsing JSON.
CREATE TABLE IF NOT EXISTS bridge_setup (
    peer_id TEXT PRIMARY KEY,
    -- The peer's role as its own message said: 'hub' (on a machine) or 'machine' (on a hub).
    peer_role TEXT NOT NULL CHECK (peer_role IN ('hub', 'machine')),
    -- The name the peer gave, already checked and cut to a slug: a host or remote MCP name.
    peer_name TEXT NOT NULL,
    -- This side's forward to the peer's service ('web' on a machine, 'gate' on a hub), picked
    -- here from 47400 to 47999 and kept.
    local_port INTEGER NOT NULL,
    -- Whether the peer offers that service ('web' from a hub, 'gate' from a machine).
    peer_offers INTEGER NOT NULL DEFAULT 0,
    -- Hub only: the machine's forward to this hub's `web`, so agents launched there call back to
    -- http://127.0.0.1:<peer_port>/mcp.
    peer_port INTEGER,
    -- Machine only: the per-device MCP key the hub minted for it. The hub keeps only a hash,
    -- in bridge_device_keys.
    mcp_key TEXT,
    -- Machine only: whether it offered its gate when it sent its message, so offering one later
    -- runs the setup again.
    offered INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The hub's per-device MCP keys, hashed (sha256, hex). A live key opens /mcp with full scope.
CREATE TABLE IF NOT EXISTS bridge_device_keys (
    key_hash TEXT PRIMARY KEY,
    peer_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX IF NOT EXISTS bridge_device_keys_peer ON bridge_device_keys (peer_id);
