// Times for reminders, worked out in the zone the owner's reminders mean (their saved zone), not the
// browser's, so "this evening" is the evening where they are. Pure: every function takes the clock.

import { zoneOffsetMinutes } from './time'

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS
const DAY_MS = 24 * HOUR_MS
const EVENING_HOUR = 18
const MORNING_HOUR = 9
/** An evening choice that is nearly now is not offered: it would ring while the person is still looking. */
const EVENING_MIN_LEAD_MS = 30 * MINUTE_MS

export interface ZoneParts {
  year: number
  month: number
  day: number
  hour: number
  minute: number
  /** 1 is Monday, 7 is Sunday. */
  weekday: number
}

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

export function zoneParts(ms: number, zone: string): ZoneParts {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
    weekday: 'short',
  }).formatToParts(ms)
  const num = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0)
  const weekday = WEEKDAYS.indexOf(parts.find((p) => p.type === 'weekday')?.value ?? 'Mon') + 1
  return { year: num('year'), month: num('month'), day: num('day'), hour: num('hour'), minute: num('minute'), weekday }
}

const HALF_HOUR_MS = 30 * MINUTE_MS

/** The instant whose wall clock in `zone` reads `wall` (given as if it were UTC), or null when no instant does. */
function exactInstant(zone: string, wall: number): number | null {
  let guess = wall
  for (let i = 0; i < 3; i++) guess = wall - zoneOffsetMinutes(zone, guess) * MINUTE_MS
  const p = zoneParts(guess, zone)
  return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute) === wall ? guess : null
}

/**
 * The instant a wall-clock time falls on in `zone`. A time a clock change skips lands just after the
 * gap, as it does on the server; one that happens twice is the first.
 */
export function epochInZone(zone: string, year: number, month: number, day: number, hour: number, minute: number): number {
  const wall = Date.UTC(year, month - 1, day, hour, minute)
  for (let step = 0; step < 4; step++) {
    const w = wall + step * HALF_HOUR_MS
    const at = exactInstant(zone, w)
    if (at === null) continue
    // A time that happens twice as the clocks go back is the first of them.
    const earlier = at - HOUR_MS
    const p = zoneParts(earlier, zone)
    return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute) === w ? earlier : at
  }
  return wall
}

const two = (n: number) => String(n).padStart(2, '0')

/** `2026-11-27T16:00`: what a `datetime-local` input holds and what `remind_create` takes as `at`. */
export function localString(ms: number, zone: string): string {
  const p = zoneParts(ms, zone)
  return `${p.year}-${two(p.month)}-${two(p.day)}T${two(p.hour)}:${two(p.minute)}`
}

/** The date and time typed in a `datetime-local` input, as an instant in `zone`; null when it is not one. */
export function parseLocalInput(text: string, zone: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(text)
  if (!m) return null
  const [year, month, day, hour, minute] = m.slice(1).map(Number)
  return epochInZone(zone, year, month, day, hour, minute)
}

function atHour(nowMs: number, zone: string, daysAhead: number, hour: number): number {
  const today = zoneParts(nowMs, zone)
  const day = zoneParts(epochInZone(zone, today.year, today.month, today.day, 12, 0) + daysAhead * DAY_MS, zone)
  return epochInZone(zone, day.year, day.month, day.day, hour, 0)
}

export type TimeChoiceKey = 'hour' | 'evening' | 'tomorrow' | 'monday'

export interface TimeChoice {
  key: TimeChoiceKey
  /** When it rings, as epoch milliseconds. */
  at: number
}

/** The quick times offered for a snooze or a new reminder: an hour, this evening, tomorrow morning, Monday. */
export function timeChoices(nowMs: number, zone: string, opts: { monday?: boolean } = {}): TimeChoice[] {
  const out: TimeChoice[] = [{ key: 'hour', at: nowMs + HOUR_MS }]
  const evening = atHour(nowMs, zone, 0, EVENING_HOUR)
  if (evening - nowMs >= EVENING_MIN_LEAD_MS) out.push({ key: 'evening', at: evening })
  out.push({ key: 'tomorrow', at: atHour(nowMs, zone, 1, MORNING_HOUR) })
  if (opts.monday) {
    const today = zoneParts(nowMs, zone)
    const ahead = ((8 - today.weekday) % 7) || 7
    out.push({ key: 'monday', at: atHour(nowMs, zone, ahead, MORNING_HOUR) })
  }
  return out
}

export type RepeatChoice = 'none' | 'daily' | 'weekdays' | 'weekly' | 'monthly'

/** The `repeat` the server takes for a choice, from the day it first rings (weekly and monthly follow that day). */
export function repeatSpec(choice: RepeatChoice, firstMs: number, zone: string): string | undefined {
  const p = zoneParts(firstMs, zone)
  switch (choice) {
    case 'none':
      return undefined
    case 'daily':
      return 'daily'
    case 'weekdays':
      return 'weekdays'
    case 'weekly':
      return `weekly:${p.weekday}`
    case 'monthly':
      return `monthly:${p.day}`
  }
}

export type ReminderGroup = 'overdue' | 'today' | 'tomorrow' | 'later'

export const GROUP_ORDER: ReminderGroup[] = ['overdue', 'today', 'tomorrow', 'later']

/** Which group an active reminder belongs to by when it next rings, in `zone`. */
export function groupOf(dueMs: number, nowMs: number, zone: string): ReminderGroup {
  if (dueMs < nowMs) return 'overdue'
  const due = zoneParts(dueMs, zone)
  const now = zoneParts(nowMs, zone)
  const dayNo = (p: ZoneParts) => Math.round(Date.UTC(p.year, p.month - 1, p.day) / DAY_MS)
  const ahead = dayNo(due) - dayNo(now)
  if (ahead <= 0) return 'today'
  if (ahead === 1) return 'tomorrow'
  return 'later'
}

export function groupReminders<T extends { due_epoch: number }>(items: T[], nowMs: number, zone: string): { group: ReminderGroup; items: T[] }[] {
  const sorted = [...items].sort((a, b) => a.due_epoch - b.due_epoch)
  return GROUP_ORDER.map((group) => ({ group, items: sorted.filter((r) => groupOf(r.due_epoch * 1000, nowMs, zone) === group) })).filter((g) => g.items.length > 0)
}
