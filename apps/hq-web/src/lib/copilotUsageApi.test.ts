import { expect, test } from 'bun:test'
import {
  chipLabel,
  exhaustionText,
  formatCompact,
  formatHours,
  formatPerHour,
  projectionText,
  type CopilotBurn,
  type CopilotUsage,
} from './copilotUsageApi'

const burn = (over: Partial<CopilotBurn> = {}): CopilotBurn => ({
  windows: [],
  projected_at_reset: 45000,
  hours_to_reset: 20,
  projected_exhaustion_at: null,
  exhausts_before_reset: false,
  overage_permitted: false,
  sample_count: 10,
  confidence: 'high',
  ...over,
})

test('formatCompact shortens thousands and millions', () => {
  expect(formatCompact(12940)).toBe('12.9k')
  expect(formatCompact(950)).toBe('950')
  expect(formatCompact(50000)).toBe('50k')
  expect(formatCompact(1_250_000)).toBe('1.3M')
})

test('formatPerHour and formatHours handle unknowns and units', () => {
  expect(formatPerHour(null)).toBe('-')
  expect(formatPerHour(1200)).toBe('1.2k/h')
  expect(formatHours(0.5)).toBe('30m')
  expect(formatHours(3.5)).toBe('3.5h')
  expect(formatHours(52)).toBe('2d 4h')
})

test('projectionText reports share of the total or missing data', () => {
  expect(projectionText(burn(), 50000)).toBe('45k of 50k (90%)')
  expect(projectionText(burn({ projected_at_reset: null }), 50000)).toBe('not enough samples yet')
})

test('exhaustionText says whether the balance outlasts the cycle', () => {
  const now = Date.parse('2026-09-30T00:00:00Z')
  const soon = burn({ projected_exhaustion_at: '2026-09-30T05:00:00Z', exhausts_before_reset: true, overage_permitted: true })
  expect(exhaustionText(soon, now)).toBe('in 5h, before reset (overage allowed)')
  expect(exhaustionText(burn(), now)).toBe('not at this rate')
})

test('chipLabel shows only for a metered active account', () => {
  const usage: CopilotUsage = {
    active: true,
    burn: burn(),
    quota: {
      login: null, plan: null, sku: null, entitlement: 50000, credits_used: 37009, remaining: 12940,
      percent_remaining: 25.8, overage_permitted: true, reset_at: null, fetched_at: '2026-09-30T00:00:00Z',
    },
  }
  expect(chipLabel(usage)).toBe('12.9k left')
  expect(chipLabel({ active: false })).toBeNull()
  expect(chipLabel({ active: true, note: 'free' })).toBeNull()
  expect(chipLabel(undefined)).toBeNull()
})
