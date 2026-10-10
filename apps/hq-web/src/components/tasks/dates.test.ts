import { afterEach, expect, setSystemTime, test } from 'bun:test'
import { formatDay, parseDay, todayDay } from './dates'

const originalTz = process.env.TZ

afterEach(() => {
  setSystemTime()
  if (originalTz === undefined) delete process.env.TZ
  else process.env.TZ = originalTz
})

test('the same instant is a different local day east and west of UTC', () => {
  // 22:30 UTC on the 9th is already the 10th in Kampala (UTC+3) and still the 9th in New York.
  const instant = new Date('2026-10-09T22:30:00Z')
  process.env.TZ = 'Africa/Kampala'
  setSystemTime(instant)
  expect(formatDay(todayDay())).toBe('2026-10-10')
  process.env.TZ = 'America/New_York'
  setSystemTime(instant)
  expect(formatDay(todayDay())).toBe('2026-10-09')
})

test('a date-only value is a whole UTC day whatever the viewer timezone', () => {
  process.env.TZ = 'Pacific/Auckland'
  const day = parseDay('2026-10-09')!
  expect(formatDay(day)).toBe('2026-10-09')
  expect(parseDay('2026-02-30')).toBeNull()
  expect(parseDay(null)).toBeNull()
})
