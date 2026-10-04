-- FTS5 for full-text search on notes
CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
    path,
    title,
    content,
    tags,
    tokenize='porter unicode61'
);

-- Vector embeddings
CREATE TABLE IF NOT EXISTS embeddings (
    note_path TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,
    model TEXT NOT NULL,
    embedded_at INTEGER NOT NULL
);

-- File sync state
CREATE TABLE IF NOT EXISTS sync_state (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    modified_at INTEGER NOT NULL,
    synced_at INTEGER NOT NULL
);

-- Distributed traces
CREATE TABLE IF NOT EXISTS traces (
    trace_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    root_instruction TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    total_tasks INTEGER DEFAULT 0,
    completed_tasks INTEGER DEFAULT 0,
    failed_tasks INTEGER DEFAULT 0,
    started_at INTEGER NOT NULL,
    completed_at INTEGER
);

CREATE TABLE IF NOT EXISTS spans (
    span_id TEXT PRIMARY KEY,
    trace_id TEXT NOT NULL REFERENCES traces(trace_id),
    parent_span_id TEXT,
    task_id TEXT,
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    claimed_by TEXT,
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS span_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    span_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    message TEXT
);

-- Advisory locks
CREATE TABLE IF NOT EXISTS locks (
    path TEXT PRIMARY KEY,
    holder TEXT NOT NULL,
    acquired_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

-- Plans
CREATE TABLE IF NOT EXISTS plans (
    plan_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS plan_steps (
    step_id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL REFERENCES plans(plan_id),
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    sort_order INTEGER NOT NULL DEFAULT 0,
    result TEXT,
    metadata TEXT
);

-- Usage / budget tracking
CREATE TABLE IF NOT EXISTS usage_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_name TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    recorded_at TEXT NOT NULL
);

-- Memory system
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    source TEXT NOT NULL,
    importance REAL NOT NULL DEFAULT 0.5,
    created_at INTEGER NOT NULL,
    last_accessed INTEGER,
    access_count INTEGER DEFAULT 0,
    consolidated INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS consolidations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    insight TEXT NOT NULL,
    memory_ids TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_spans_trace ON spans(trace_id);
CREATE INDEX IF NOT EXISTS idx_span_events_span ON span_events(span_id);
CREATE INDEX IF NOT EXISTS idx_span_events_trace ON span_events(trace_id);
CREATE INDEX IF NOT EXISTS idx_plan_steps_plan ON plan_steps(plan_id);
CREATE INDEX IF NOT EXISTS idx_usage_agent ON usage_records(agent_name);
CREATE INDEX IF NOT EXISTS idx_usage_recorded ON usage_records(recorded_at);
CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance DESC);
CREATE INDEX IF NOT EXISTS idx_sync_modified ON sync_state(modified_at);
