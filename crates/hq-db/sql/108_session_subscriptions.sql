-- Subscriptions: any session may watch another at run time, for chosen event kinds.
-- NULL kinds keeps the old meaning (finished, blocked and exited).
ALTER TABLE harness_session_observers ADD COLUMN kinds TEXT;
-- How many relays deep a session's current turn is: a relay-written message carries the
-- depth its writer had plus one, and delivering it hands that depth to the recipient. Input
-- from anyone else resets it, so only chains of agents waking each other are capped.
ALTER TABLE task_comments ADD COLUMN relay_depth INTEGER NOT NULL DEFAULT 0;
ALTER TABLE harness_sessions ADD COLUMN relay_depth INTEGER NOT NULL DEFAULT 0;
