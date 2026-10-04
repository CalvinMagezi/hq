import { expect, test } from 'bun:test'
import { relTime } from './time'

test('relTime buckets by minute, hour and day', () => {
  const ago = (ms: number) => Date.now() - ms
  expect(relTime(undefined)).toBe('')
  expect(relTime(ago(5_000))).toBe('just now')
  expect(relTime(ago(5 * 60_000))).toBe('5m ago')
  expect(relTime(new Date(ago(3 * 3_600_000)).toISOString())).toBe('3h ago')
  expect(relTime(new Date(ago(2 * 86_400_000)))).toBe('2d ago')
  expect(relTime('not a date')).toBe('not a date')
})
