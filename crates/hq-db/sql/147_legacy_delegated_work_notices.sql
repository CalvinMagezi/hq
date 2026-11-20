-- Notices raised before the plain wording (PR 665) carry a run id, a role and a tool name in their
-- stored text, and 165 of them were still waiting in Needs you. Rows keep what they were written
-- with, so this fixes them once. It is safe to run again: it only touches rows still pending
-- or still carrying the old text.
--
-- 1. A notice about a run that is settled and already reviewed, or older than a day, is dismissed:
--    it leaves Needs you and stays in Done. A notice whose run row is gone counts as settled.
UPDATE value_items
SET state = 'dismissed',
    engagement = COALESCE(engagement, 'dismissed'),
    engaged_at = COALESCE(engaged_at, strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    engagement_mac = NULL
WHERE source_task = 'subagent_runs'
  AND dedup_key LIKE 'subagent-run-%'
  AND state IN ('pending', 'routed', 'delivered')
  AND (
    EXISTS (
      SELECT 1 FROM subagent_runs r
      WHERE 'subagent-run-' || r.run_id = value_items.dedup_key
        AND r.exec_status NOT IN ('queued', 'running')
        AND r.reviewed_at IS NOT NULL
    )
    OR (
      julianday(created_at) < julianday('now') - 1
      AND NOT EXISTS (
        SELECT 1 FROM subagent_runs r
        WHERE 'subagent-run-' || r.run_id = value_items.dedup_key
          AND r.exec_status IN ('queued', 'running')
      )
    )
  );

-- 2. Whatever is left, and every dismissed copy, gets the plain wording.
UPDATE value_items
SET title = 'A background task did not finish',
    body = 'A background task did not finish. Open the chat to see what is missing.'
WHERE source_task = 'subagent_runs'
  AND dedup_key LIKE 'subagent-run-%'
  AND (title = 'Delegated work needs attention'
       OR body LIKE 'Sub-agent run %'
       OR body LIKE '%subagent_run_status%');
