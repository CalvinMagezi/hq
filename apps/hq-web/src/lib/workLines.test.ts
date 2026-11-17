import { describe, expect, test } from 'bun:test'
import { helpersLine, tasksLine } from './workLines'

const task = (status: string, archived = false) => ({ status, archived_at: archived ? '2026-01-01' : null }) as never

describe('the Work rows', () => {
  test('say how many are open and how many wait on the person', () => {
    expect(tasksLine([task('to_do'), task('in_progress'), task('ready_for_review'), task('blocked')])).toBe('4 open · 2 waiting on you')
    expect(tasksLine([task('to_do'), task('complete'), task('to_do', true)])).toBe('1 open · none waiting on you')
    expect(tasksLine([])).toBe('Nothing open')
  })
  test('count helpers that are running, and say when none wait', () => {
    const s = (status: string, archived = false) => ({ status, archived, reachable: true, agent_status: 'working', pending_wake: null }) as never
    expect(helpersLine([s('running'), s('exited')])).toBe('1 working here · none need you')
    expect(helpersLine([])).toBe('None working here')
  })
})
