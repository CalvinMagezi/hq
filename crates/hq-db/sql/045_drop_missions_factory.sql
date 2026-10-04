-- The durable mission engine and the software factory scout it fed are
-- retired: HQ no longer uses autonomous missions to improve its own codebase
-- or any other codebase. Single-user instance, no external readers of this
-- data, so it drops rather than staying as an unread historical table.
DROP TABLE IF EXISTS mission_events;
DROP TABLE IF EXISTS mission_steps;
DROP TABLE IF EXISTS missions;
DROP TABLE IF EXISTS factory_proposals;
