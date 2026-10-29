-- A host with running sessions that stopped answering the session supervisor
-- (docs/plans/remote-driven-teams.md, phase 5.1). One row per host while it is silent,
-- so a daemon restart neither forgets the outage nor alerts about it twice.
CREATE TABLE IF NOT EXISTS host_outages (
    host TEXT PRIMARY KEY,
    -- Unix seconds of the first poll that failed.
    since INTEGER NOT NULL,
    -- The last poll's error, for the alert.
    detail TEXT NOT NULL DEFAULT '',
    -- The "not answering" inbox item, once raised.
    alert_item_id TEXT
);
