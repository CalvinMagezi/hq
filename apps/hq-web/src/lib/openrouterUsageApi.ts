import { queryOptions } from '@tanstack/react-query'
import { hqJson } from './hqAuth'

export interface OpenRouterUsage {
  usage: number
  usage_daily: number | null
  usage_weekly: number | null
  usage_monthly: number | null
  limit: number | null
  limit_remaining: number | null
  limit_reset: string | null
  is_free_tier: boolean
  total_credits: number | null
  total_usage: number | null
}

export interface OpenRouterUsageResponse {
  active: boolean
  model?: string | null
  usage?: OpenRouterUsage
  credits_left?: number | null
  unavailable?: boolean
  note?: string
  error?: string
}

export const OPENROUTER_USAGE_POLL_MS = 60_000

export const openrouterUsageQuery = queryOptions({
  queryKey: ['openrouter-usage'],
  queryFn: (): Promise<OpenRouterUsageResponse> => hqJson('/api/openrouter-usage'),
  refetchInterval: OPENROUTER_USAGE_POLL_MS,
})

const SMALL_USD = 10

/** USD with cents; unknown shows a dash. Sub-dollar-tenths keep three places so small spend is not "$0.00". */
export function formatUsd(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '-'
  if (n !== 0 && Math.abs(n) < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(Math.abs(n) < SMALL_USD ? 3 : 2).replace(/(\.\d\d)0$/, '$1')}`
}

/** Header chip label: what is left under a cap or credit balance, else today's spend. Null when nothing is known. */
export function openrouterChipLabel(r: OpenRouterUsageResponse | undefined): string | null {
  const u = r?.usage
  if (!r?.active || !u) return null
  const left = u.limit_remaining ?? r.credits_left ?? null
  if (left !== null) return `${formatUsd(left)} left`
  return u.usage_daily === null ? null : `${formatUsd(u.usage_daily)} today`
}

export function openrouterChipTitle(r: OpenRouterUsageResponse | undefined): string {
  const u = r?.usage
  if (!u) return 'OpenRouter spend'
  return `OpenRouter: ${formatUsd(u.usage_daily)} today, ${formatUsd(u.usage_weekly)} this week, ${formatUsd(u.usage_monthly)} this month`
}
