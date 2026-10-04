import { expect, test } from 'bun:test'
import { formatSeconds } from './settingsApi'

test('formatSeconds picks the largest exact unit', () => {
  expect(formatSeconds(21600)).toBe('6 h')
  expect(formatSeconds(270)).toBe('270 s')
  expect(formatSeconds(300)).toBe('5 min')
  expect(formatSeconds(45)).toBe('45 s')
})
