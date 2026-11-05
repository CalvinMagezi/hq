import { useSyncExternalStore } from 'react'
import { t, tn } from '~/strings'
import { deviceTimeZone, isValidTimeZone, readClockZone, zoneCity } from './clock'

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

/** The zone times show in: the one chosen in this browser, else the browser's own. */
export function displayZone(): string {
  return readClockZone() || deviceTimeZone()
}

function zoneParts(ms: number, zone: string) {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
    second: 'numeric',
  }).formatToParts(ms)
  const get = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0)
  return { year: get('year'), month: get('month'), day: get('day'), hour: get('hour'), minute: get('minute'), second: get('second') }
}

/** Minutes `zone` is ahead of UTC at `ms`. */
export function zoneOffsetMinutes(zone: string, ms: number): number {
  const p = zoneParts(ms, zone)
  const asUtc = Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second)
  return Math.round((asUtc - Math.floor(ms / 1000) * 1000) / MINUTE_MS)
}

/** The calendar day of `ms` in `zone`, as a day count. */
const dayNumber = (ms: number, zone: string) => {
  const p = zoneParts(ms, zone)
  return Math.round(Date.UTC(p.year, p.month - 1, p.day) / DAY_MS)
}

/** "24 Oct", or "24 Oct 2025" for another year; the browser's own order and language, in the display zone. */
function dayLabel(ms: number, nowMs: number, zone: string): string {
  const sameYear = zoneParts(ms, zone).year === zoneParts(nowMs, zone).year
  return new Intl.DateTimeFormat(undefined, { timeZone: zone, day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) }).format(ms)
}

interface HqZone {
  tz: string | null
  offsetMinutes: number | null
}

let hq: HqZone = { tz: null, offsetMinutes: null }
const hqListeners = new Set<() => void>()

/** HQ's own zone, from every hello. */
export function noteHqTimeZone(hello: { tz?: string | null; tz_offset_minutes?: number }) {
  const tz = isValidTimeZone(hello.tz) ? hello.tz : null
  const offsetMinutes = typeof hello.tz_offset_minutes === 'number' ? hello.tz_offset_minutes : null
  if (tz === hq.tz && offsetMinutes === hq.offsetMinutes) return
  hq = { tz, offsetMinutes }
  for (const l of hqListeners) l()
}

/** HQ's IANA zone, or null while unknown. */
export function hqZone(): string | null {
  return hq.tz
}

export function useHqZone(): string | null {
  return useSyncExternalStore(
    (cb) => {
      hqListeners.add(cb)
      return () => void hqListeners.delete(cb)
    },
    hqZone,
    () => null,
  )
}

/** Whether HQ's zone and the display zone tell a different clock time right now. False while HQ's zone is unknown. */
export function zonesDiffer(hqTz: string | null = hqZone(), display: string = displayZone(), nowMs: number = Date.now()): boolean {
  if (!hqTz) return false
  return zoneOffsetMinutes(hqTz, nowMs) !== zoneOffsetMinutes(display, nowMs)
}

/** "07:00" for a time HQ keeps in its own zone: with the city ("07:00 Nairobi time") only when the zones differ. */
export function namedTime(time: string, hqTz: string | null = hqZone(), display: string = displayZone(), nowMs: number = Date.now()): string {
  return hqTz && zonesDiffer(hqTz, display, nowMs) ? t('tz_named_time', { time, city: zoneCity(hqTz) }) : time
}

/** Midnight UTC, the moment spending limits reset, as a clock time in `zone`. */
export function utcMidnightIn(zone: string, nowMs: number = Date.now()): string {
  const midnight = Math.floor(nowMs / DAY_MS) * DAY_MS
  return new Intl.DateTimeFormat(undefined, { timeZone: zone, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(midnight)
}

/** Spending's reset line: always names UTC and adds the display zone's time. */
export function spendResetLine(zone: string = displayZone(), nowMs: number = Date.now()): string {
  return t('spend_reset_line', { time: utcMidnightIn(zone, nowMs), city: zoneCity(zone) })
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
export function dateByAge(when: string | number | Date, nowMs: number = Date.now(), zone: string = displayZone()): string {
  const ms = parse(when)
  if (ms === null) return t('time_unknown')
  const days = dayNumber(nowMs, zone) - dayNumber(ms, zone)
  if (days <= 0) return new Intl.DateTimeFormat(undefined, { timeZone: zone, hour: 'numeric', minute: '2-digit' }).format(ms)
  if (days === 1) return t('time_yesterday')
  return dayLabel(ms, nowMs, zone)
}
