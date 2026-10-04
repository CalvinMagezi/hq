import { expect, test } from 'bun:test'
import { groupByInitiative, nestRows, compareTasksByActivity } from './hierarchy'
import type { Initiative, TaskItem } from '~/lib/tasksApi'

const makeTask = (id: string, display_id: string, initiative_id: string, updated_at: string, created_at = updated_at): TaskItem => ({
  id,
  display_id,
  initiative_id,
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

test('compareTasksByActivity orders newest activity first with deterministic tie-breaking', () => {
  const tOld = makeTask('1', 'T-001', 'init-1', '2026-09-18 10:00:00')
  const tMidA = makeTask('2', 'T-002', 'init-1', '2026-09-28 10:00:00', '2026-09-20 10:00:00')
  const tMidB = makeTask('3', 'T-003', 'init-1', '2026-09-28 10:00:00', '2026-09-25 10:00:00')
  const tNew = makeTask('4', 'T-004', 'init-1', '2026-09-28 21:30:00')

  const list = [tOld, tMidA, tNew, tMidB].sort(compareTasksByActivity)
  expect(list.map((t) => t.display_id)).toEqual(['T-004', 'T-003', 'T-002', 'T-001'])
})

test('groupByInitiative orders initiative groups by newest task activity and tasks within group by activity', () => {
  const initiatives = new Map<string, Initiative>([
    ['init-a', { id: 'init-a', space_id: 's', folder_id: null, name: 'Active Development', slug: 'active', id_prefix: 'SHADOW' }],
    ['init-b', { id: 'init-b', space_id: 's', folder_id: null, name: 'Target HQ', slug: 'thq', id_prefix: 'FR' }],
  ])

  // In Active Development, tasks are 10 days old
  const shadow001 = makeTask('s1', 'SHADOW-001', 'init-a', '2026-09-18 12:00:00')
  const shadow003 = makeTask('s3', 'SHADOW-003', 'init-a', '2026-09-18 14:00:00')

  // In Target HQ, task is 57 minutes old (globally newest)
  const fr059 = makeTask('f59', 'FR-059', 'init-b', '2026-09-28 20:33:00')
  const fr050 = makeTask('f50', 'FR-050', 'init-b', '2026-09-20 10:00:00')

  const tasks = [shadow001, shadow003, fr050, fr059]
  const grouped = groupByInitiative(tasks, initiatives)

  // Target HQ has the newest task (FR-059 from 2026-09-28 20:33:00), so Target HQ must be group[0]!
  expect(grouped[0][0]).toBe('Target HQ')
  expect(grouped[0][1].map((t) => t.display_id)).toEqual(['FR-059', 'FR-050'])

  // Active Development has older tasks, so it must be group[1], with SHADOW-003 before SHADOW-001
  expect(grouped[1][0]).toBe('Active Development')
  expect(grouped[1][1].map((t) => t.display_id)).toEqual(['SHADOW-003', 'SHADOW-001'])
})

test('nestRows orders subtasks by newest activity first', () => {
  const parent = makeTask('p1', 'P-001', 'init-a', '2026-09-20 10:00:00')
  const childOld = { ...makeTask('c1', 'C-001', 'init-a', '2026-09-22 10:00:00'), parent_task_id: 'p1' }
  const childNew = { ...makeTask('c2', 'C-002', 'init-a', '2026-09-28 10:00:00'), parent_task_id: 'p1' }

  const byId = new Map([
    ['p1', parent],
    ['c1', childOld],
    ['c2', childNew],
  ])

  const rows = nestRows([parent, childOld, childNew], byId)
  expect(rows.map((r) => `${r.depth}:${r.task.display_id}`)).toEqual([
    '0:P-001',
    '1:C-002',
    '1:C-001',
  ])
})
