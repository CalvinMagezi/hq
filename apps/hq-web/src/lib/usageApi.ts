import { queryOptions } from '@tanstack/react-query'
import { hqJson } from './hqAuth'

export type BudgetState = 'ok' | 'warning' | 'exceeded'
export type BudgetPeriod = 'day' | 'week' | 'month'
export type BudgetAction = 'block' | 'downgrade' | 'notify'
export type Confidence = 'low' | 'medium' | 'high'

export interface BudgetStatus {
  name: string
  scope: string
  period: BudgetPeriod
  action: BudgetAction
  limit_usd: number
  spent_usd: number
  pct: number
  state: BudgetState
  period_start: number
  resets_at: number
}

export interface BudgetDef {
  name: string
  scope: string
  period: BudgetPeriod
  limit_usd: number
  soft_pct: number[]
  action: BudgetAction
  downgrade_model?: string | null
}

export interface BudgetsResponse {
  budgets: BudgetStatus[]
  /** The budgets as configured, so an edit keeps fields the status does not carry. */
  definitions: BudgetDef[]
  background_run_usd: number | null
  allow_unpriced_models: string[]
  problems: string[]
}

export interface ForecastWindow {
  window_hours: number
  usd: number
  per_hour: number
}

export interface Forecast {
  windows: ForecastWindow[]
  rate_per_hour: number | null
  spent_this_period: number
  hours_to_reset: number
  projected_period_total: number | null
  limit_usd: number | null
  hours_to_limit: number | null
  projected_exhaustion_at: string | null
  exhausts_before_reset: boolean
  confidence: Confidence
  history_hours: number
  seasonality_note: string | null
}

export interface Driver {
  name: string
  usd: number
  calls: number
  avg_input_tokens: number
  cache_hit_rate: number | null
}

export interface ForecastResponse {
  month: Forecast
  budgets: { budget: string; scope: string; forecast: Forecast }[]
  drivers: { models: Driver[]; origins: Driver[] }
}

export interface LedgerWindows {
  today: number
  week: number
  month: number
  unpriced_calls: number
}

export interface ProviderReading {
  balance: { amount: number; currency: string } | null
  spend: { today: number | null; week: number | null; month: number | null } | null
  limit_remaining: number | null
}

export interface RateLimitReading {
  requests_limit: number | null
  requests_remaining: number | null
  requests_reset: string | null
  tokens_limit: number | null
  tokens_remaining: number | null
  tokens_reset: string | null
  captured_at: number
}

export interface ProviderRow {
  backend: string
  adapter: string
  source: 'provider' | 'ledger'
  status: 'ok' | 'refused' | 'error' | 'no_key' | 'local' | 'subscription' | 'ledger_only'
  provider: ProviderReading | null
  ledger: LedgerWindows
  rate_limit: RateLimitReading | null
  note: string | null
}

export interface ProvidersResponse {
  generated_at: string
  backends: ProviderRow[]
}

export const USAGE_POLL_MS = 60_000

export const budgetsQuery = queryOptions({
  queryKey: ['usage-budgets'],
  queryFn: (): Promise<BudgetsResponse> => hqJson('/api/budgets'),
  refetchInterval: USAGE_POLL_MS,
})

export const forecastQuery = queryOptions({
  queryKey: ['usage-forecast'],
  queryFn: (): Promise<ForecastResponse> => hqJson('/api/usage/forecast'),
  refetchInterval: USAGE_POLL_MS,
})

export const providersQuery = queryOptions({
  queryKey: ['usage-providers'],
  queryFn: (): Promise<ProvidersResponse> => hqJson('/api/usage/providers'),
  refetchInterval: USAGE_POLL_MS,
})

export function saveBudgets(defs: BudgetDef[], rest: Pick<BudgetsResponse, 'background_run_usd' | 'allow_unpriced_models'>) {
  return hqJson<{ saved: number }>('/api/budgets', 'PUT', {
    budgets: defs,
    background_run_usd: rest.background_run_usd,
    allow_unpriced_models: rest.allow_unpriced_models,
  })
}

const SMALL_USD = 10

export function usd(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '-'
  if (n !== 0 && Math.abs(n) < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(Math.abs(n) < SMALL_USD ? 3 : 2).replace(/(\.\d\d)0$/, '$1')}`
}

export function pct(n: number | null | undefined): string {
  return n === null || n === undefined || !Number.isFinite(n) ? '-' : `${Math.round(n)}%`
}

const HOURS_PER_DAY = 24

export const PERIOD_ADVERB: Record<BudgetPeriod, string> = { day: 'daily', week: 'weekly', month: 'monthly' }

/** A span of hours as the largest sensible unit: "6h", "3 days". */
export function span(hours: number | null | undefined): string {
  if (hours === null || hours === undefined || !Number.isFinite(hours)) return '-'
  if (hours < 1) return '<1h'
  if (hours < HOURS_PER_DAY) return `${Math.round(hours)}h`
  const days = Math.round(hours / HOURS_PER_DAY)
  return `${days} day${days === 1 ? '' : 's'}`
}

/** Where an exhaustion forecast stands, in words, never presenting a low-confidence guess as fact. */
export function exhaustionSentence(f: Forecast): string | null {
  if (f.hours_to_limit === null || f.limit_usd === null) return null
  const when = `${usd(f.limit_usd)} is used up in about ${span(f.hours_to_limit)}`
  const relation = f.exhausts_before_reset ? 'before the period resets' : 'after the period resets'
  const hedge = f.confidence === 'low' ? ' (low confidence: little history)' : ''
  return `${when}, ${relation}${hedge}`
}

/** The budget closest to its limit that needs attention, as a header chip label. */
export function budgetChipLabel(r: BudgetsResponse | undefined): string | null {
  const worst = [...(r?.budgets ?? [])]
    .filter((b) => b.state !== 'ok')
    .sort((a, b) => b.pct - a.pct)[0]
  return worst ? `${worst.name} ${pct(worst.pct)}` : null
}

export function budgetChipTitle(r: BudgetsResponse | undefined): string {
  const worst = [...(r?.budgets ?? [])].filter((b) => b.state !== 'ok').sort((a, b) => b.pct - a.pct)[0]
  return worst
    ? `Budget ${worst.name}: ${usd(worst.spent_usd)} of ${usd(worst.limit_usd)} ${PERIOD_ADVERB[worst.period]} on ${worst.scope}`
    : 'Budgets'
}
