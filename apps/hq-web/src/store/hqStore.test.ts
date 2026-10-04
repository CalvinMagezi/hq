import { expect, test } from 'bun:test'
import { useHQStore } from './hqStore'
import type { TaskItem } from '~/lib/tasksApi'

const makeTask = (id: string, display_id: string, updated_at: string, created_at = updated_at): TaskItem => ({
  id,
  display_id,
  initiative_id: 'init-1',
  title: `Task ${display_id}`,
  description: '',
  status: 'in_progress',
  priority: 'normal',
  due_date: null,
  start_date: null,
  parent_task_id: null,
  tags: [],
  depends_on: [],
  blocked_by: [],
  subtask_count: 0,
  subtask_done: 0,
  work_started_at: null,
  first_ready_for_review_at: null,
  created_by: 'test',
  created_at,
  updated_at,
})

test('setTasks sorts tasks by latest activity descending with deterministic tie breaks', () => {
  const t1 = makeTask('1', 'T-001', '2026-09-18 10:00:00')
  const t2 = makeTask('2', 'T-002', '2026-09-28 10:00:00', '2026-09-20 10:00:00')
  const t3 = makeTask('3', 'T-003', '2026-09-28 10:00:00', '2026-09-25 10:00:00')
  const t4 = makeTask('4', 'T-004', '2026-09-28 21:00:00')

  useHQStore.getState().setTasks([t1, t2, t4, t3])
  const tasks = useHQStore.getState().tasks
  expect(tasks.map((t) => t.display_id)).toEqual(['T-004', 'T-003', 'T-002', 'T-001'])
})

test('upsertTask inserts or updates while maintaining activity-descending order', () => {
  const t1 = makeTask('1', 'T-001', '2026-09-18 10:00:00')
  const t2 = makeTask('2', 'T-002', '2026-09-28 10:00:00')
  useHQStore.getState().setTasks([t1, t2])

  // Insert newest task
  const tNew = makeTask('3', 'T-003', '2026-09-29 08:00:00')
  useHQStore.getState().upsertTask(tNew)
  expect(useHQStore.getState().tasks.map((t) => t.display_id)).toEqual(['T-003', 'T-002', 'T-001'])

  // Update older task so it becomes newest
  const t1Updated = { ...t1, updated_at: '2026-09-29 12:00:00' }
  useHQStore.getState().upsertTask(t1Updated)
  expect(useHQStore.getState().tasks.map((t) => t.display_id)).toEqual(['T-001', 'T-003', 'T-002'])
})
