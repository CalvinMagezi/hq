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

import { displayZone, namedTime, noteHqTimeZone, spendResetLine, utcMidnightIn, zonesDiffer } from './time'

test('check 1: HQ in Nairobi, browser in London: a daily watch names its zone, an ordinary time has no city', () => {
  const now = Date.UTC(2026, 10, 5, 12, 0, 0)
  expect(namedTime('07:00', 'Africa/Nairobi', 'Europe/London', now)).toBe('07:00 Nairobi time')
  expect(namedTime('07:00', 'Africa/Nairobi', 'Africa/Nairobi', now)).toBe('07:00')
  expect(namedTime('07:00', null, 'Europe/London', now)).toBe('07:00')
  // A message time is formatted in the display zone and never carries a city.
  const message = dateByAge(Date.UTC(2026, 10, 5, 11, 2), now, 'Europe/London')
  expect(message).not.toMatch(/Nairobi|London/)
  expect(zonesDiffer('Africa/Nairobi', 'Africa/Kampala', now)).toBe(false)
})

test('HQ reports its zone and a junk zone is ignored', () => {
  noteHqTimeZone({ tz: 'Africa/Nairobi', tz_offset_minutes: 180 })
  noteHqTimeZone({ tz: 'Mars/Olympus', tz_offset_minutes: 180 })
  expect(typeof displayZone()).toBe('string')
})

test('check 2: Spending names UTC and the local time of the reset', () => {
  const now = Date.UTC(2026, 10, 5, 12, 0, 0)
  expect(utcMidnightIn('Africa/Nairobi', now)).toBe('03:00')
  expect(spendResetLine('Africa/Nairobi', now)).toBe('Limits reset at midnight UTC, which is 03:00 in Nairobi. Weeks start on Monday.')
})

test('dateByAge decides today and yesterday in the display zone', () => {
  const now = Date.UTC(2026, 10, 5, 22, 30)
  // 22:30 UTC is already 01:30 on the 6th in Nairobi, so 20:00 UTC on the 5th is yesterday there.
  expect(dateByAge(Date.UTC(2026, 10, 5, 20, 0), now, 'Africa/Nairobi')).toBe('yesterday')
  expect(dateByAge(Date.UTC(2026, 10, 5, 20, 0), now, 'UTC')).toMatch(/\d/)
})
