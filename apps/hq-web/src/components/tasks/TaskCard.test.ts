import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { StaleIdsContext } from './staleContext'
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
  blocked_reason: null,
  waiting_on: null,
  blocked_since: null,
  long_horizon: false,
  archived_at: null,
  created_by: 'alice',
  created_at: '2026-09-28 10:00:00',
  updated_at: '2026-09-28 20:30:00',
  ...overrides,
})

test('TaskCard renders with defensive width and overflow classes for mobile safety', () => {
  const html = renderToString(createElement(TaskCard, { task: makeTask() }))

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

test('a stale task, a long running one and a blocked reason are each named on the card', () => {
  const stale = new Set(['task-1'])
  const html = renderToString(
    createElement(
      StaleIdsContext.Provider,
      { value: stale },
      createElement(TaskCard, {
        task: makeTask({ status: 'blocked', blocked_reason: 'no staging key', waiting_on: 'ops', long_horizon: true, blocked_by: [] }),
      })
    )
  )
  expect(html).toContain('stale')
  expect(html).toContain('long running')
  expect(html).toContain('no staging key')
})

test('a card for a task nobody flagged shows none of those chips', () => {
  const html = renderToString(createElement(TaskCard, { task: makeTask({ blocked_by: [], tags: [] }) }))
  expect(html).not.toContain('>stale<')
  expect(html).not.toContain('long running')
})

test('a poll that finds the same stale tasks keeps the same set, and a changed one does not', async () => {
  const { stableSet } = await import('./staleContext')
  const before = new Set(['a', 'b'])
  expect(stableSet(new Set(['b', 'a']), before)).toBe(before)
  const changed = new Set(['a', 'c'])
  expect(stableSet(changed, before)).toBe(changed)
  const smaller = new Set(['a'])
  expect(stableSet(smaller, before)).toBe(smaller)
})
