import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { asksBeforeDone, dependentsOf } from './doneAsk'
import type { TaskItem } from './tasksApi'

const mk = (id: string, over: Partial<TaskItem> = {}) =>
  ({ id, status: 'in_progress', depends_on: [], archived_at: null, ...over }) as unknown as TaskItem

describe('tasks wave 4', () => {
  test('P3: one done asks when other open tasks wait on it', () => {
    const a = mk('a')
    const all = [a, mk('b', { depends_on: ['a'] }), mk('c', { depends_on: ['a'], status: 'complete' })]
    expect(dependentsOf(a, all).length).toBe(1)
    expect(asksBeforeDone(a, { status: 'complete' }, all)).toBe(1)
  })
  test('P3: no question without dependents or for other changes', () => {
    const a = mk('a')
    expect(asksBeforeDone(a, { status: 'complete' }, [a])).toBe(0)
    expect(asksBeforeDone(a, { title: 'x' }, [a, mk('b', { depends_on: ['a'] })])).toBe(0)
  })
  test('board has a Move control that is not drag only', () => {
    const src = readFileSync('src/components/tasks/TaskBoardView.tsx', 'utf8')
    expect(src).toContain('tasks_move_to')
  })
})
