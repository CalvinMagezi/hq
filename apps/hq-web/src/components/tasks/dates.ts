// Task dates are plain `YYYY-MM-DD` calendar days. Working in whole UTC days
// keeps a bar from shifting by one when the viewer's timezone is behind UTC.

const MS_PER_DAY = 86_400_000
const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/

/** Day number (days since 1970-01-01 UTC), or null for anything that isn't a real date. */
export function parseDay(value: string | null | undefined): number | null {
  if (!value || !DATE_PATTERN.test(value)) return null
  const ms = Date.parse(`${value}T00:00:00Z`)
  return Number.isNaN(ms) ? null : Math.round(ms / MS_PER_DAY)
}

export function formatDay(day: number): string {
  return new Date(day * MS_PER_DAY).toISOString().slice(0, 10)
}

export function todayDay(): number {
  const now = new Date()
  return Math.round(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate()) / MS_PER_DAY)
}

export function dayOfWeek(day: number): number {
  return new Date(day * MS_PER_DAY).getUTCDay()
}

export function dayOfMonth(day: number): number {
  return new Date(day * MS_PER_DAY).getUTCDate()
}

export function isWeekend(day: number): boolean {
  const dow = dayOfWeek(day)
  return dow === 0 || dow === 6
}

export function monthLabel(day: number): string {
  return new Date(day * MS_PER_DAY).toLocaleDateString(undefined, { month: 'short', year: 'numeric', timeZone: 'UTC' })
}

export function shortLabel(value: string): string {
  const day = parseDay(value)
  if (day === null) return value
  return new Date(day * MS_PER_DAY).toLocaleDateString(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' })
}
