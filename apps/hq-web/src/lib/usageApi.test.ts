import { expect, test } from 'bun:test'
import { PERIOD_ADVERB, budgetChipLabel, exhaustionSentence, pct, span, usd, type BudgetsResponse, type Forecast } from './usageApi'

const budget = (name: string, p: number, state: 'ok' | 'warning' | 'exceeded') => ({
  name, scope: 'global', period: 'month' as const, action: 'block' as const, limit_usd: 20,
  spent_usd: (20 * p) / 100, pct: p, state, period_start: 0, resets_at: 1,
})
const resp = (...bs: ReturnType<typeof budget>[]): BudgetsResponse => ({
  budgets: bs, definitions: [], background_run_usd: null, allow_unpriced_models: [], problems: [],
})
const forecast = (over: Partial<Forecast> = {}): Forecast => ({
  windows: [], rate_per_hour: 0.5, spent_this_period: 10, hours_to_reset: 400, projected_period_total: 210,
  limit_usd: 20, hours_to_limit: 20, projected_exhaustion_at: null, exhausts_before_reset: true,
  confidence: 'medium', history_hours: 48, seasonality_note: null, ...over,
})

test('money keeps tiny spend visible and dashes the unknown', () => {
  expect(usd(0.0042)).toBe('$0.0042')
  expect(usd(37.5)).toBe('$37.50')
  expect(usd(null)).toBe('-')
  expect(pct(84.6)).toBe('85%')
})

test('periods read as adverbs', () => {
  expect(PERIOD_ADVERB.day).toBe('daily')
  expect(PERIOD_ADVERB.month).toBe('monthly')
})

test('spans pick a readable unit', () => {
  expect(span(0.2)).toBe('<1h')
  expect(span(6)).toBe('6h')
  expect(span(72)).toBe('3 days')
  expect(span(24)).toBe('1 day')
})

test('the chip names the budget closest to its limit and stays quiet when all are fine', () => {
  expect(budgetChipLabel(resp(budget('a', 10, 'ok'), budget('b', 85, 'warning'), budget('c', 120, 'exceeded')))).toBe('c 120%')
  expect(budgetChipLabel(resp(budget('a', 10, 'ok')))).toBeNull()
  expect(budgetChipLabel(undefined)).toBeNull()
})

test('an exhaustion forecast says when, relative to the reset, and hedges when history is thin', () => {
  expect(exhaustionSentence(forecast())).toBe('$20.00 is used up in about 20h, before the period resets')
  expect(exhaustionSentence(forecast({ confidence: 'low', exhausts_before_reset: false }))).toContain('low confidence')
  expect(exhaustionSentence(forecast({ hours_to_limit: null }))).toBeNull()
})
