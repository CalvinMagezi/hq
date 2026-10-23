-- The `hq edge` this device is registered with (docs/security/PAIRING.md, docs/EDGE.md).
--
-- One row at most: a device is reached through one public forwarder, and the connector's public
-- name is derived from it. Written only after the person at the edge typed the right digits.
-- The edge is not a paired peer: it may open streams on the edge protocol and nothing else, and a
-- paired peer may not open them. Clearing the row cuts the edge's open connections.
CREATE TABLE IF NOT EXISTS bridge_edge (
    slot INTEGER PRIMARY KEY CHECK (slot = 1),
    -- The edge's iroh endpoint id, lowercase hex: the only peer allowed on the edge protocol.
    id TEXT NOT NULL,
    -- The domain the edge serves devices under, and the name this device is reached at.
    domain TEXT NOT NULL,
    hostname TEXT NOT NULL,
    -- The relay the edge offered, when it has one.
    relay TEXT,
    -- Unix seconds.
    registered_at INTEGER NOT NULL
);
