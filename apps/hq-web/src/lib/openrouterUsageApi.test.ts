import { expect, test } from 'bun:test'
import { formatUsd, openrouterChipLabel, type OpenRouterUsage, type OpenRouterUsageResponse } from './openrouterUsageApi'

const usage = (over: Partial<OpenRouterUsage> = {}): OpenRouterUsage => ({
  usage: 12.5, usage_daily: 0.75, usage_weekly: 4, usage_monthly: 12.5, limit: null,
  limit_remaining: null, limit_reset: null, is_free_tier: false, total_credits: null, total_usage: null, ...over,
})
const resp = (u: OpenRouterUsage, extra: Partial<OpenRouterUsageResponse> = {}): OpenRouterUsageResponse => ({ active: true, usage: u, ...extra })

test('formatUsd shows cents, keeps tiny spend visible and dashes the unknown', () => {
  expect(formatUsd(37.5)).toBe('$37.50')
  expect(formatUsd(0.75)).toBe('$0.75')
  expect(formatUsd(0.0042)).toBe('$0.0042')
  expect(formatUsd(0)).toBe('$0.00')
  expect(formatUsd(null)).toBe('-')
})

test('the chip prefers what is left, falls back to today, and hides when unknown', () => {
  expect(openrouterChipLabel(resp(usage({ limit_remaining: 37.5 })))).toBe('$37.50 left')
  expect(openrouterChipLabel(resp(usage(), { credits_left: 70 }))).toBe('$70.00 left')
  expect(openrouterChipLabel(resp(usage({ limit_remaining: 5 }), { credits_left: 70 }))).toBe('$5.00 left')
  expect(openrouterChipLabel(resp(usage()))).toBe('$0.75 today')
  expect(openrouterChipLabel(resp(usage({ usage_daily: null })))).toBeNull()
  expect(openrouterChipLabel({ active: false })).toBeNull()
  expect(openrouterChipLabel({ active: true, unavailable: true })).toBeNull()
})
