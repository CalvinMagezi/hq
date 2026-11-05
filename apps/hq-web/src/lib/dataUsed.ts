// Data used by HQ in this browser this month (B17). The counting itself is in dataUsage.ts; the
// counter starts over on the 1st, in the display zone.

import { deviceTimeZone, readClockZone } from './clock'

const KEY = 'hq.dataUsed'

interface Counter {
  month: string
  bytes: number
}

/** "2026-11" for `now` in `zone`. */
export function monthKey(now: Date, zone: string): string {
  const parts = new Intl.DateTimeFormat('en-GB', { timeZone: zone, year: 'numeric', month: '2-digit' }).formatToParts(now)
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? ''
  return `${get('year')}-${get('month')}`
}

function thisMonth(now = new Date()): string {
  return monthKey(now, readClockZone() || deviceTimeZone())
}

/** Bytes used this month: a counter from an earlier month counts as zero. */
export function readDataUsed(now = new Date()): number {
  try {
    const c = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Counter | null
    return c && c.month === thisMonth(now) ? c.bytes : 0
  } catch {
    return 0
  }
}

export function addDataUsed(bytes: number, now = new Date()) {
  if (!(bytes > 0)) return
  try {
    localStorage.setItem(KEY, JSON.stringify({ month: thisMonth(now), bytes: readDataUsed(now) + bytes }))
  } catch {
    // Storage can be blocked; the number then only reads zero.
  }
}
