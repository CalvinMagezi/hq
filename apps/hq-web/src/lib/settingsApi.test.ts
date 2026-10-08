import { expect, test } from 'bun:test'
import { formatSeconds, normalizeSettings } from './settingsApi'

test('formatSeconds picks the largest exact unit', () => {
  expect(formatSeconds(21600)).toBe('6 h')
  expect(formatSeconds(270)).toBe('270 s')
  expect(formatSeconds(300)).toBe('5 min')
  expect(formatSeconds(45)).toBe('45 s')
})

const HOST = { default_host: 'local', hosts: [], sandbox_mode: 'process', sandbox_extra_domains: 0, idle_reap_hours: 0, drive_new_watches: false, driver_checkin_minutes: 5, driver_nudge_budget: 3, driver_no_progress_limit: 2 }
// Only the fields the normalizer touches matter here.
const base = {} as Parameters<typeof normalizeSettings>[0]

test('a gateway that still serves the old herdr section does not leave agent_host undefined', () => {
  expect(normalizeSettings({ ...base, herdr: HOST }).agent_host).toEqual(HOST)
})

test('a gateway with no coding-agent section yields null, not undefined', () => {
  expect(normalizeSettings(base).agent_host).toBeNull()
})

test('the current agent_host section wins over a stale herdr one', () => {
  expect(normalizeSettings({ ...base, agent_host: HOST, herdr: { ...HOST, default_host: 'old' } }).agent_host?.default_host).toBe('local')
})
