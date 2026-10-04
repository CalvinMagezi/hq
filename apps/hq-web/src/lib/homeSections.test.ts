import { expect, test } from 'bun:test'
import { renderToString } from 'react-dom/server'
import { createElement } from 'react'
import { HOME_TASK_LIMIT, inProgressTasks, sectionState } from './homeSections'
import { HomeSection } from '~/components/vault/HomeSection'
import { HomeSections } from '~/components/vault/HomeSections'
import type { TaskItem } from './tasksApi'

const task = (id: string, status: TaskItem['status'], updated: string): TaskItem =>
  ({ id, display_id: id, status, updated_at: updated, created_at: updated, title: id }) as TaskItem

test('sectionState covers loading, empty, error and ready', () => {
  const base = { data: undefined, isPending: false, isError: false, error: null }
  expect(sectionState({ ...base, isPending: true })).toEqual({ kind: 'loading' })
  expect(sectionState({ ...base, data: [] })).toEqual({ kind: 'empty' })
  expect(sectionState({ ...base, isError: true, error: new Error('boom') })).toEqual({ kind: 'error', message: 'boom' })
  expect(sectionState({ ...base, isError: true, error: 'x' })).toEqual({ kind: 'error', message: 'Request failed' })
  expect(sectionState({ ...base, data: [1, 2] })).toEqual({ kind: 'ready', items: [1, 2] })
})

test('saved data stays visible when a refresh fails', () => {
  const s = sectionState({ data: [1], isPending: false, isError: true, error: new Error('offline') })
  expect(s.kind).toBe('ready')
})

test('inProgressTasks keeps only in-progress tasks, newest activity first, capped', () => {
  const tasks = [
    task('a', 'in_progress', '2026-09-01 10:00:00'),
    task('b', 'complete', '2026-09-05 10:00:00'),
    task('c', 'in_progress', '2026-09-03 10:00:00'),
    task('d', 'to_do', '2026-09-04 10:00:00'),
  ]
  expect(inProgressTasks(tasks).map((t) => t.id)).toEqual(['c', 'a'])
  const many = Array.from({ length: HOME_TASK_LIMIT + 3 }, (_, i) => task(`t${i}`, 'in_progress', '2026-09-01 10:00:00'))
  expect(inProgressTasks(many)).toHaveLength(HOME_TASK_LIMIT)
})

const html = (state: Parameters<typeof HomeSection>[0]['state']) =>
  renderToString(
    createElement(HomeSection, { title: 'Pinned', state, emptyText: 'Nothing pinned yet.', onRetry: () => {}, render: () => null }),
  )

test('HomeSection renders loading, empty and error states', () => {
  expect(html({ kind: 'loading' })).toContain('Loading Pinned')
  expect(html({ kind: 'empty' })).toContain('Nothing pinned yet.')
  const err = html({ kind: 'error', message: 'offline' })
  expect(err).toContain('Could not load pinned: offline')
  expect(err).toContain('Retry')
})

test('HomeSections orders pinned, recent, tasks and an empty section keeps its siblings', () => {
  const noop = () => {}
  const out = renderToString(
    createElement(HomeSections, {
      pinned: { kind: 'empty' },
      recent: { kind: 'loading' },
      tasks: { kind: 'error', message: 'down' },
      onRetryPinned: noop,
      onRetryRecent: noop,
      onRetryTasks: noop,
    }),
  )
  const at = (t: string) => out.indexOf(`data-home-section="${t}"`)
  expect(at('Pinned')).toBeGreaterThanOrEqual(0)
  expect(at('Pinned')).toBeLessThan(at('Recent Project Notes'))
  expect(at('Recent Project Notes')).toBeLessThan(at('In Progress Tasks'))
  expect(out).toContain('No pinned notes yet')
  expect(out).toContain('Could not load in progress tasks: down')
})
