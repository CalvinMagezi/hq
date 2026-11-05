import { t, tn } from '~/strings'

const MINUTE_MS = 60_000
const DAY_MS = 86_400_000
const MINUTES_PER_HOUR = 60
const HOURS_PER_DAY = 24
const DAYS_PER_WEEK = 7

function parse(when?: string | number | Date): number | null {
  if (when === undefined || when === '') return null
  const ms = new Date(when).getTime()
  return Number.isNaN(ms) ? null : ms
}

const startOfDay = (ms: number) => new Date(ms).setHours(0, 0, 0, 0)

/** "24 Oct", or "24 Oct 2025" for another year; the browser's own order and language. */
function dayLabel(ms: number, nowMs: number): string {
  const sameYear = new Date(ms).getFullYear() === new Date(nowMs).getFullYear()
  return new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) }).format(ms)
}

/** Whole phrase for a span: "now", "14 min ago", "2 h ago", "4 days ago". Callers never add "ago". */
export function relTime(when?: string | number | Date, nowMs: number = Date.now()): string {
  if (when === undefined || when === '') return ''
  const ms = parse(when)
  if (ms === null) return typeof when === 'string' ? when : ''
  const minutes = Math.max(0, Math.floor((nowMs - ms) / MINUTE_MS))
  if (minutes < 1) return t('time_just_now')
  if (minutes < MINUTES_PER_HOUR) return t('time_min_ago', { n: minutes })
  const hours = Math.floor(minutes / MINUTES_PER_HOUR)
  if (hours < HOURS_PER_DAY) return t('time_hour_ago', { n: hours })
  return tn('time_days_ago', Math.floor(hours / HOURS_PER_DAY))
}

/** The compact form for rows: "now", "3m", "1h", "1d", "2w". */
export function shortAge(when?: string | number | Date, nowMs: number = Date.now()): string {
  const ms = parse(when)
  if (ms === null) return ''
  const minutes = Math.max(0, Math.floor((nowMs - ms) / MINUTE_MS))
  if (minutes < 1) return t('time_just_now')
  if (minutes < MINUTES_PER_HOUR) return t('time_short_min', { n: minutes })
  const hours = Math.floor(minutes / MINUTES_PER_HOUR)
  if (hours < HOURS_PER_DAY) return t('time_short_hour', { n: hours })
  const days = Math.floor(hours / HOURS_PER_DAY)
  return days < DAYS_PER_WEEK ? t('time_short_day', { n: days }) : t('time_short_week', { n: Math.floor(days / DAYS_PER_WEEK) })
}

/** A moment by its age: a clock time today, "yesterday", then a date. Never a bare clock time for an older day. */
export function dateByAge(when: string | number | Date, nowMs: number = Date.now()): string {
  const ms = parse(when)
  if (ms === null) return t('time_unknown')
  const days = Math.round((startOfDay(nowMs) - startOfDay(ms)) / DAY_MS)
  if (days <= 0) return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(ms)
  if (days === 1) return t('time_yesterday')
  return dayLabel(ms, nowMs)
}
