import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import type { TaskItem } from '~/lib/tasksApi'
import { TaskCard } from './TaskCard'
import { TaskBoardView } from './TaskBoardView'
import { TaskFormModal } from './TaskFormModal'
import { todayGroups } from './TaskListView'
import { weekRange } from './gantt'

const task = (id: string, status: TaskItem['status']): TaskItem => ({
  id, display_id: id, initiative_id: null, title: `Task ${id}`, description: '', status, priority: null,
  due_date: null, start_date: null, parent_task_id: null, assignees: [], tags: [], depends_on: [], blocked_by: [],
  subtask_count: 0, subtask_done: 0, work_started_at: null, first_ready_for_review_at: null, completed_at: null,
  estimate_minutes: null, blocked_reason: null, waiting_on: null, blocked_since: null, long_horizon: false,
  archived_at: null, created_by: 'alice', created_at: '2026-09-28 10:00:00', updated_at: '2026-09-28 10:00:00',
})

const root = join(import.meta.dir, '..', '..')
const files = (dir: string): string[] =>
  readdirSync(dir).flatMap((n) => {
    const p = join(dir, n)
    return statSync(p).isDirectory() ? files(p) : [p]
  })

// web-matches-phone B3: the Tasks place on the aligned design.
describe('tasks, aligned with mobile', () => {
  test('a task is a design row with its own done check', () => {
    const html = renderToStaticMarkup(createElement(TaskCard, { task: task('a', 'to_do') }))
    expect(html).toContain('class="a-trow"')
    expect(html).toContain('class="a-chk"')
    expect(html).not.toContain('hq-card')
  })

  test('Today groups what waits on you, what is moving, what is due and what is done', () => {
    const groups = todayGroups([task('a', 'to_do'), task('b', 'blocked'), task('c', 'in_progress'), task('d', 'complete')])
    expect(groups.map(([name]) => name)).toEqual(['Waiting on you', 'In progress', 'Due or starting today', 'Done today'])
  })

  test('a board column is a design column', () => {
    const html = renderToStaticMarkup(createElement(TaskBoardView, { tasks: [task('a', 'blocked')], allById: new Map(), onSelect: () => undefined, onMove: () => undefined }))
    expect(html).toContain('bd-col needs')
  })

  test('Week is two weeks from this Monday', () => {
    const monday = 20_000 - ((new Date(20_000 * 86_400_000).getUTCDay() + 6) % 7)
    expect(weekRange(20_000)).toEqual({ start: monday, end: monday + 13 })
  })

  test('New task opens the shared sheet', () => {
    const html = renderToStaticMarkup(
      createElement(TaskFormModal, { open: true, onClose: () => undefined, spaces: [], folders: [], initiatives: [], tasks: [], onTaxonomyChanged: async () => undefined, onCreate: async () => undefined }),
    )
    expect(html).toContain('New task')
    expect(html).toContain('a-sheet')
  })

  test('the owned files use the design and tokens, not the old look', () => {
    const owned = [...files(join(root, 'components/tasks')), join(root, 'routes/tasks.tsx')].filter((f) => /\.(tsx|css)$/.test(f) && !f.includes('.test.'))
    const banned = /emerald|rose-|amber-|neutral-|hq-card|hq-glass|hq-modal|hq-field|rgba\(|fixed inset-0/
    expect(owned.filter((f) => banned.test(readFileSync(f, 'utf8')))).toEqual([])
  })
})
