CREATE TABLE IF NOT EXISTS workflow_runs (
  run_id          TEXT PRIMARY KEY,
  workflow_name   TEXT NOT NULL,
  status          TEXT NOT NULL,
  started_at      INTEGER NOT NULL,
  finished_at     INTEGER,
  duration_ms     INTEGER,
  error           TEXT,
  trigger_payload TEXT
);

CREATE INDEX IF NOT EXISTS idx_runs_wf
  ON workflow_runs(workflow_name, started_at DESC);

CREATE TABLE IF NOT EXISTS workflow_run_events (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id      TEXT NOT NULL REFERENCES workflow_runs(run_id),
  seq         INTEGER NOT NULL,
  kind        TEXT NOT NULL,
  step_id     TEXT,
  ts          INTEGER,
  duration_ms INTEGER,
  output      TEXT,
  error       TEXT,
  UNIQUE (run_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_events_run
  ON workflow_run_events(run_id, seq);
