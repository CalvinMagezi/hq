import { expect, test } from 'bun:test'
import { workingNow } from './workingContext'
import type { WorkSession } from '~/lib/tasksApi'

const session = (task_id: string, actor: string, ended_at: string | null): WorkSession => ({
  id: `${task_id}-${actor}`,
  task_id,
  actor,
  harness: '',
  host: '',
  cwd: '',
  branch: '',
  harness_session_id: null,
  started_at: '2026-10-09 01:00:00',
  last_heartbeat_at: '2026-10-09 01:00:00',
  ended_at,
  end_reason: null,
  active_seconds: 0,
})

test('only sessions that have not ended count as working now', () => {
  const live = workingNow([session('a', 'alpha', null), session('b', 'beta', '2026-10-09 02:00:00')])
  expect([...live]).toEqual([['a', 'alpha']])
})
