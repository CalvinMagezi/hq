import { expect, test } from 'bun:test'
import { renderToString } from 'react-dom/server'
import { TaskCard } from './TaskCard'
import type { TaskItem } from '~/lib/tasksApi'

const makeTask = (overrides: Partial<TaskItem> = {}): TaskItem => ({
  id: 'task-1',
  display_id: 'SHADOW-001',
  initiative_id: 'init-1',
  title: 'A very long task title that might overflow narrow screens if not styled with break words',
  description: 'Detailed description with unbroken strings https://github.com/example/really/long/url/that/could/overflow/mobile/containers',
  status: 'ready_for_review',
  priority: 'urgent',
  due_date: '2026-10-01',
  start_date: '2026-09-20',
  parent_task_id: null,
  tags: ['frontend', 'mobile-responsive-super-long-tag-name-that-tests-truncation'],
  depends_on: [],
  blocked_by: ['SHADOW-002', 'SHADOW-003', 'SHADOW-004'],
  subtask_count: 5,
  subtask_done: 3,
  work_started_at: null,
  first_ready_for_review_at: null,
  completed_at: null,
  estimate_minutes: null,
  created_by: 'alice',
  created_at: '2026-09-28 10:00:00',
  updated_at: '2026-09-28 20:30:00',
  ...overrides,
})

test('TaskCard renders with defensive width and overflow classes for mobile safety', () => {
  const html = renderToString(TaskCard({ task: makeTask() }))

  // Outer container must constrain width and prevent horizontal overflow
  expect(html).toContain('w-full')
  expect(html).toContain('max-w-full')
  expect(html).toContain('min-w-0')
  expect(html).toContain('overflow-hidden')

  // Header must wrap on small screens
  expect(html).toContain('flex-wrap')

  // Title and description must break words
  expect(html).toContain('break-words')

  // Blocked chips and tags must truncate inside max-w-full container
  expect(html).toContain('truncate')
  expect(html).toContain('SHADOW-002, SHADOW-003, SHADOW-004')
})
