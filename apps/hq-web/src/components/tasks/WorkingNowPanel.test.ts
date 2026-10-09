import { expect, test } from 'bun:test'
import { elapsedSeconds, formatClock } from './WorkingNowPanel'
import type { WorkSession } from '~/lib/tasksApi'

test('the clock reads minutes and seconds, then hours', () => {
  expect(formatClock(0)).toBe('0:00')
  expect(formatClock(270)).toBe('4:30')
  expect(formatClock(3729)).toBe('1:02:09')
  expect(formatClock(-5)).toBe('0:00')
})

test('elapsed time counts from the stored UTC start', () => {
  const started = { started_at: '2026-10-09 01:00:00' } as WorkSession
  const at = Date.parse('2026-10-09T01:01:30Z')
  expect(elapsedSeconds(started, at)).toBe(90)
})
