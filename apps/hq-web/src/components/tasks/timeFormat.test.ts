import { expect, test } from 'bun:test'
import { actualSpans, formatDuration, formatSignedMinutes } from './timeFormat'
import type { WorkSession } from '~/lib/tasksApi'

test('durations show the two largest units', () => {
  expect(formatDuration(0)).toBe('0s')
  expect(formatDuration(45)).toBe('45s')
  expect(formatDuration(12 * 60 + 30)).toBe('12m')
  expect(formatDuration(2 * 3600 + 5 * 60)).toBe('2h 5m')
  expect(formatDuration(2 * 3600)).toBe('2h')
  expect(formatDuration(3 * 86_400 + 4 * 3600 + 59)).toBe('3d 4h')
})

test('an unknown duration says so rather than showing zero', () => {
  expect(formatDuration(null)).toBe('unknown')
  expect(formatDuration(undefined)).toBe('unknown')
  expect(formatDuration(-5)).toBe('0s')
})

test('a variance keeps its sign', () => {
  expect(formatSignedMinutes(60)).toBe('+1h')
  expect(formatSignedMinutes(-30)).toBe('-30m')
  expect(formatSignedMinutes(0)).toBe('0s')
})

const session = (over: Partial<WorkSession>): WorkSession => ({
  id: 'ws-1',
  task_id: 'tk-1',
  actor: 'a',
  harness: '',
  host: '',
  cwd: '',
  branch: '',
  harness_session_id: null,
  started_at: '2026-10-01 10:00:00',
  last_heartbeat_at: '2026-10-01 10:00:00',
  ended_at: '2026-10-01 12:00:00',
  end_reason: 'released',
  active_seconds: 7200,
  ...over,
})

test('a lease becomes a UTC day span and a live one is marked', () => {
  const spans = actualSpans([
    session({}),
    session({ id: 'ws-2', started_at: '2026-10-01 23:00:00', active_seconds: 7200, ended_at: null }),
  ])
  const [first, second] = spans.get('tk-1')!
  expect(first.end - first.start).toBe(0)
  expect(first.live).toBe(false)
  expect(second.end - second.start).toBe(1)
  expect(second.live).toBe(true)
})

test('a lease with an unreadable start is skipped', () => {
  expect(actualSpans([session({ started_at: 'garbage' })]).size).toBe(0)
})
