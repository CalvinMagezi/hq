-- Clean up remaining legacy delegated-work notices that survived migration 147
-- (settled runs that were unreviewed and younger than a day) and collapse any
-- duplicate open delegated-work alerts to a single newest row. Only dismisses,
-- never deletes. Safe to run again.
--
-- 1. Dismiss every open delegated-work notice whose run is settled (any age)
--    or missing. Alerts for runs that are still queued or running stay open.
UPDATE value_items
SET state = 'dismissed',
    engagement = COALESCE(engagement, 'dismissed'),
    engaged_at = COALESCE(engaged_at, strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    engagement_mac = NULL
WHERE source_task = 'subagent_runs'
  AND dedup_key LIKE 'subagent-run-%'
  AND state IN ('pending', 'routed', 'delivered')
  AND NOT EXISTS (
    SELECT 1 FROM subagent_runs r
    WHERE 'subagent-run-' || r.run_id = value_items.dedup_key
      AND r.settled_at IS NULL
      AND r.exec_status IN ('queued', 'running')
  );

-- 2. Collapse identical open delegated-work alerts to one (keep the newest,
--    dismiss the rest).
UPDATE value_items
SET state = 'dismissed',
    engagement = COALESCE(engagement, 'dismissed'),
    engaged_at = COALESCE(engaged_at, strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    engagement_mac = NULL
WHERE id IN (
  SELECT id FROM (
    SELECT id,
           ROW_NUMBER() OVER (
             PARTITION BY source_task, kind, title, body, COALESCE(link, '')
             ORDER BY created_at DESC, rowid DESC
           ) AS rn
    FROM value_items
    WHERE source_task = 'subagent_runs'
      AND state IN ('pending', 'routed', 'delivered')
  )
  WHERE rn > 1
);

-- 3. Collapse any remaining duplicate open alerts for the same run (dedup_key),
--    keeping only the newest.
UPDATE value_items
SET state = 'dismissed',
    engagement = COALESCE(engagement, 'dismissed'),
    engaged_at = COALESCE(engaged_at, strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    engagement_mac = NULL
WHERE id IN (
  SELECT id FROM (
    SELECT id,
           ROW_NUMBER() OVER (
             PARTITION BY kind, dedup_key
             ORDER BY created_at DESC, rowid DESC
           ) AS rn
    FROM value_items
    WHERE source_task = 'subagent_runs'
      AND dedup_key IS NOT NULL
      AND state IN ('pending', 'routed', 'delivered')
  )
  WHERE rn > 1
);
