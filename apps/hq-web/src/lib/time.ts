const MINUTE_MS = 60_000
const MINUTES_PER_HOUR = 60
const HOURS_PER_DAY = 24

/** "just now", "5m ago", "3h ago" or "2d ago"; an unparseable string comes back as is. */
export function relTime(when?: string | number | Date): string {
  if (when === undefined || when === '') return ''
  const t = new Date(when).getTime()
  if (Number.isNaN(t)) return typeof when === 'string' ? when : ''
  const m = Math.floor((Date.now() - t) / MINUTE_MS)
  if (m < 1) return 'just now'
  if (m < MINUTES_PER_HOUR) return `${m}m ago`
  const h = Math.floor(m / MINUTES_PER_HOUR)
  if (h < HOURS_PER_DAY) return `${h}h ago`
  return `${Math.floor(h / HOURS_PER_DAY)}d ago`
}
