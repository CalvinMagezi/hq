// The data budget (web wave 2, spec 03 rules 8 and 9): bytes counted per hour and per month in this
// browser only, the first load of Chat, and the data saver. Nothing here is ever sent anywhere.

import { addDataUsed } from './dataUsed'
import { knownInHub } from './hubWindow'

const HOURS_KEY = 'hq.dataHours'
const FIRST_LOAD_KEY = 'hq.dataFirstLoad'
const KEPT_HOURS = 48
const HOUR_MS = 3_600_000
const MB = 1_000_000
/** What HQ aims for while a tab sits open and idle. */
export const IDLE_TARGET_BYTES = MB
/** What a Chat first load should stay under. */
export const FIRST_LOAD_TARGET_BYTES = 300_000
const SAVER_SEARCH_DEBOUNCE_MS = 300

interface HourBucket {
  /** Hours since the epoch, UTC: a bucket is a clock hour. */
  hour: number
  bytes: number
}

const hourOf = (now: Date) => Math.floor(now.getTime() / HOUR_MS)

function readHours(): HourBucket[] {
  try {
    const v = JSON.parse(localStorage.getItem(HOURS_KEY) ?? '[]') as HourBucket[]
    return Array.isArray(v) ? v : []
  } catch {
    return []
  }
}

/** Counts `bytes` against this hour and this month. */
export function addBytes(bytes: number, now = new Date()) {
  if (!(bytes > 0)) return
  addDataUsed(bytes, now)
  const hour = hourOf(now)
  const hours = readHours()
  const last = hours[hours.length - 1]
  if (last?.hour === hour) last.bytes += bytes
  else hours.push({ hour, bytes })
  try {
    localStorage.setItem(HOURS_KEY, JSON.stringify(hours.slice(-KEPT_HOURS)))
  } catch {
    // Storage can be blocked; the numbers then only read zero.
  }
}

/** A socket frame has no transfer size, so its text length stands for it. */
export function addSocketBytes(length: number) {
  addBytes(length)
}

/** Bytes of the last whole hour with any counting, or null before one has passed. */
export function lastHourBytes(now = new Date()): number | null {
  const current = hourOf(now)
  const done = readHours().filter((h) => h.hour < current)
  return done.length ? done[done.length - 1].bytes : null
}

export function overIdleTarget(bytes: number): boolean {
  return bytes > IDLE_TARGET_BYTES
}

interface TransferEntry {
  name: string
  transferSize?: number
}

/** Transfer sizes of HQ's own requests (same origin) added up. */
export function sumTransfer(entries: readonly TransferEntry[], origin: string): number {
  return entries.reduce((n, e) => (e.name.startsWith(origin) ? n + (e.transferSize ?? 0) : n), 0)
}

/** What Chat's first load transferred, as last measured in this browser; null before one. */
export function readFirstLoad(): number | null {
  try {
    const v = Number(localStorage.getItem(FIRST_LOAD_KEY))
    return v > 0 ? v : null
  } catch {
    return null
  }
}

function measureFirstLoad() {
  const path = location.pathname
  if (path !== '/' && !path.startsWith('/chat')) return
  const bytes = sumTransfer([...performance.getEntriesByType('navigation'), ...performance.getEntriesByType('resource')] as TransferEntry[], location.origin)
  if (bytes <= 0) return
  try {
    localStorage.setItem(FIRST_LOAD_KEY, String(bytes))
  } catch {
    // Storage can be blocked.
  }
}

/** The browser's own data saver (Chromium only), or the phone being the hub. */
export function dataSaverOn(): boolean {
  const conn = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection
  return conn?.saveData === true || knownInHub()
}

/** Search waits a little longer after the last key when data is short. */
export function searchDebounceMs(normal: number, saver = dataSaverOn()): number {
  return saver ? Math.max(normal, SAVER_SEARCH_DEBOUNCE_MS) : normal
}

let started = false

/** Starts counting HQ's own requests once per page load, and measures the first load. */
export function startDataCounter(): void {
  if (started || typeof PerformanceObserver === 'undefined') return
  started = true
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) {
      const r = e as PerformanceResourceTiming
      if (r.name.startsWith(location.origin)) addBytes(r.transferSize)
    }
  }).observe({ type: 'resource', buffered: true })
  if (document.readyState === 'complete') setTimeout(measureFirstLoad, 0)
  else window.addEventListener('load', () => setTimeout(measureFirstLoad, 0), { once: true })
}
