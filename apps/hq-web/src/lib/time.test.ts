import { expect, test } from 'bun:test'
import { dateByAge, relTime, shortAge } from './time'

const NOW = new Date(2026, 9, 29, 15, 0, 0).getTime()
const ago = (ms: number) => NOW - ms
const MIN = 60_000
const DAY = 86_400_000

test('relTime is the whole phrase, buckets by minute, hour and day', () => {
  expect(relTime(undefined, NOW)).toBe('')
  expect(relTime(ago(5_000), NOW)).toBe('now')
  expect(relTime(ago(14 * MIN), NOW)).toBe('14 min ago')
  expect(relTime(new Date(ago(3 * 60 * MIN)).toISOString(), NOW)).toBe('3 h ago')
  expect(relTime(new Date(ago(2 * DAY)), NOW)).toBe('2 days ago')
  expect(relTime(ago(25 * 60 * MIN), NOW)).toBe('1 day ago')
  expect(relTime('not a date', NOW)).toBe('not a date')
})

test('shortAge is the compact row form', () => {
  expect(shortAge(ago(10_000), NOW)).toBe('now')
  expect(shortAge(ago(3 * MIN), NOW)).toBe('3m')
  expect(shortAge(ago(60 * MIN), NOW)).toBe('1h')
  expect(shortAge(ago(DAY), NOW)).toBe('1d')
  expect(shortAge(ago(15 * DAY), NOW)).toBe('2w')
})

test('dateByAge: a clock time today, yesterday, a date, a date with the year', () => {
  expect(dateByAge(ago(30 * MIN), NOW)).toMatch(/\d/)
  expect(dateByAge(ago(DAY), NOW)).toBe('yesterday')
  expect(dateByAge(ago(12 * DAY), NOW)).toBe(new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' }).format(ago(12 * DAY)))
  expect(dateByAge(ago(400 * DAY), NOW)).toMatch(/2025/)
  expect(dateByAge('nonsense', NOW)).toBe('Not known')
})
