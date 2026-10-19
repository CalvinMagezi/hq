import { expect, test } from 'bun:test'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { isShelved, shelveNotice, withoutShelved } from './shelf'

const list = (id: string, shelved_at: string | null = null): Initiative => ({
  id,
  space_id: 's1',
  folder_id: null,
  name: id,
  slug: id,
  id_prefix: id.toUpperCase(),
  shelved_at,
})

const task = (id: string, extra: Partial<TaskItem> = {}): TaskItem =>
  ({ id, display_id: id, initiative_id: 'l1', parent_task_id: null, shelved_at: null, ...extra }) as TaskItem

const byId = <T extends { id: string }>(rows: T[]) => new Map(rows.map((r) => [r.id, r]))

test('a task is archived by itself, by its list or by its parent', () => {
  const lists = byId([list('l1'), list('l2', '2026-10-01 00:00:00')])
  const parent = task('p', { shelved_at: '2026-10-01 00:00:00' })
  const rows = [
    task('plain'),
    task('own', { shelved_at: '2026-10-01 00:00:00' }),
    task('in_list', { initiative_id: 'l2' }),
    task('child', { parent_task_id: 'p' }),
    parent,
  ]
  const tasks = byId(rows)
  const flags = Object.fromEntries(rows.map((t) => [t.id, isShelved(t, lists, tasks)]))
  expect(flags).toEqual({ plain: false, own: true, in_list: true, child: true, p: true })
})

test('the everyday list keeps only what is not archived', () => {
  const lists = byId([list('l1')])
  const shown = withoutShelved([task('a'), task('b', { shelved_at: '2026-10-01 00:00:00' })], lists)
  expect(shown.map((t) => t.id)).toEqual(['a'])
})

test('the notice says what changed and names what was left alone', () => {
  const name = (ref: string) => ref.toUpperCase()
  expect(shelveNotice({ changed: ['a', 'b'], skipped: [] }, 'Archived', name)).toBe('Archived 2 tasks.')
  expect(shelveNotice({ changed: ['a'], skipped: [] }, 'Unarchived', name)).toBe('Unarchived 1 task.')
  expect(
    shelveNotice({ changed: [], skipped: [{ task: 'x', reason: 'alice is working on it' }] }, 'Archived', name)
  ).toBe('Nothing archived. Left alone: X alice is working on it.')
  const many = Array.from({ length: 5 }, (_, i) => ({ task: `t${i}`, reason: 'already archived' }))
  expect(shelveNotice({ changed: ['a'], skipped: many }, 'Archived', name)).toContain('and 2 more')
})
