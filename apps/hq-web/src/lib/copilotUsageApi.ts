import { queryOptions } from '@tanstack/react-query'
import { hqJson } from './hqAuth'

interface CopilotQuota {
  login: string | null
  plan: string | null
  sku: string | null
  entitlement: number
  credits_used: number
  remaining: number
  percent_remaining: number
  overage_permitted: boolean
  reset_at: string | null
  fetched_at: string
}

interface BurnWindow {
  window_hours: number
  credits_used: number | null
  per_hour: number | null
}

export interface CopilotBurn {
  windows: BurnWindow[]
  projected_at_reset: number | null
  hours_to_reset: number | null
  projected_exhaustion_at: string | null
  exhausts_before_reset: boolean
  overage_permitted: boolean
  sample_count: number
  confidence: 'low' | 'medium' | 'high'
}

export interface CopilotUsage {
  active: boolean
  quota?: CopilotQuota
  burn?: CopilotBurn
  samples?: { ts: string; credits_used: number }[]
  note?: string
  error?: string
}

export const COPILOT_USAGE_POLL_MS = 60_000

export const copilotUsageQuery = queryOptions({
  queryKey: ['copilot-usage'],
  queryFn: (): Promise<CopilotUsage> => hqJson('/api/copilot-usage'),
  refetchInterval: COPILOT_USAGE_POLL_MS,
})

const THOUSAND = 1000
const MILLION = 1_000_000
const MINUTES_PER_HOUR = 60
const HOURS_PER_DAY = 24
const TRIM_THRESHOLD = 100

/** 12940 -> "12.9k", 950 -> "950", 1_250_000 -> "1.3M". */
export function formatCompact(n: number): string {
  const abs = Math.abs(n)
  if (abs >= MILLION) return `${trim(n / MILLION)}M`
  if (abs >= THOUSAND) return `${trim(n / THOUSAND)}k`
  return String(Math.round(n))
}

function trim(v: number): string {
  return Math.abs(v) >= TRIM_THRESHOLD ? String(Math.round(v)) : v.toFixed(1).replace(/\.0$/, '')
}

/** Burn rate as "1.2k/h"; unknown rates show a dash. */
export function formatPerHour(rate: number | null | undefined): string {
  return rate === null || rate === undefined ? '-' : `${formatCompact(rate)}/h`
}

/** Hours as "45m", "3.5h" or "2d 4h". */
export function formatHours(hours: number | null | undefined): string {
  if (hours === null || hours === undefined || !Number.isFinite(hours)) return '-'
  if (hours < 1) return `${Math.max(1, Math.round(hours * MINUTES_PER_HOUR))}m`
  if (hours < HOURS_PER_DAY) return `${trim(hours)}h`
  const days = Math.floor(hours / HOURS_PER_DAY)
  return `${days}d ${Math.round(hours - days * HOURS_PER_DAY)}h`
}

/** Text for the projected use at the cycle reset, against the total allowance. */
export function projectionText(burn: CopilotBurn | undefined, total: number): string {
  if (!burn || burn.projected_at_reset === null) return 'not enough samples yet'
  const projected = burn.projected_at_reset
  const pct = total > 0 ? Math.round((projected / total) * 100) : 0
  return `${formatCompact(projected)} of ${formatCompact(total)} (${pct}%)`
}

/** Text for when the balance runs out at the current rate. */
export function exhaustionText(burn: CopilotBurn | undefined, now: number = Date.now()): string {
  if (!burn || burn.projected_exhaustion_at === null) return 'not at this rate'
  const hours = (new Date(burn.projected_exhaustion_at).getTime() - now) / (THOUSAND * 3600)
  const eta = `in ${formatHours(Math.max(0, hours))}`
  if (burn.exhausts_before_reset) return `${eta}, before reset${burn.overage_permitted ? ' (overage allowed)' : ''}`
  return `${eta}, after reset`
}

/** Header chip label, or null when there is nothing metered to show. */
export function chipLabel(usage: CopilotUsage | undefined): string | null {
  if (!usage?.active || !usage.quota || !usage.burn) return null
  return `${formatCompact(usage.quota.remaining)} left`
}

export function chipTitle(usage: CopilotUsage | undefined): string {
  const q = usage?.quota
  if (!q) return 'Copilot credits'
  const rate = usage?.burn?.windows.find((w) => w.window_hours === 6)?.per_hour
  return `Copilot credits: ${formatCompact(q.remaining)} of ${formatCompact(q.entitlement)} left, ${formatPerHour(rate)} over 6h`
}
