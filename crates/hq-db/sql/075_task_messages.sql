-- A message between agent sessions is a task comment with a kind and a
-- recipient, so the task thread stays the one durable record. `delivered_at`
-- marks when the supervisor typed it into the recipient's pane.
ALTER TABLE task_comments ADD COLUMN kind TEXT NOT NULL DEFAULT 'comment';
ALTER TABLE task_comments ADD COLUMN sender_session_id TEXT;
ALTER TABLE task_comments ADD COLUMN to_session_id TEXT;
ALTER TABLE task_comments ADD COLUMN reply_to INTEGER;
ALTER TABLE task_comments ADD COLUMN delivered_at TEXT;
CREATE INDEX IF NOT EXISTS idx_task_comments_inbox
    ON task_comments(to_session_id, delivered_at) WHERE to_session_id IS NOT NULL;
