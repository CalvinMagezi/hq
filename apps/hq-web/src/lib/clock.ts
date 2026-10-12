import { useSyncExternalStore } from 'react'

/** localStorage key for the header clock's time zone. Empty or missing means this device's zone. */
export const CLOCK_ZONE_KEY = 'hq.clock.timeZone'
const CLOCK_ZONE_EVENT = 'hq:clock-zone'

/** Zones offered when the browser cannot list its own. */
const FALLBACK_ZONES = [
  'UTC',
  'Africa/Kampala',
  'Africa/Nairobi',
  'Africa/Lagos',
  'Africa/Johannesburg',
  'Europe/London',
  'Europe/Berlin',
  'Asia/Dubai',
  'Asia/Kolkata',
  'Asia/Singapore',
  'Asia/Tokyo',
  'Australia/Sydney',
  'America/New_York',
  'America/Chicago',
  'America/Los_Angeles',
]

export function isValidTimeZone(zone: unknown): zone is string {
  if (typeof zone !== 'string' || zone === '') return false
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: zone })
    return true
  } catch {
    return false
  }
}

/** The zone this browser runs in, or UTC when it will not say. */
export function deviceTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

/** Every IANA zone the browser knows, UTC included, sorted. */
export function availableTimeZones(): string[] {
  let zones: string[]
  try {
    zones = Intl.supportedValuesOf('timeZone')
  } catch {
    zones = FALLBACK_ZONES
  }
  return [...new Set(['UTC', ...zones])].sort()
}

/** The chosen zone, or '' to follow the device. A stored zone the browser rejects counts as unset. */
export function readClockZone(): string {
  try {
    const stored = localStorage.getItem(CLOCK_ZONE_KEY) ?? ''
    return isValidTimeZone(stored) ? stored : ''
  } catch {
    return ''
  }
}

/** Pin the clock to `zone`, or pass '' to follow the device again. */
export function setClockZone(zone: string) {
  try {
    if (zone) localStorage.setItem(CLOCK_ZONE_KEY, zone)
    else localStorage.removeItem(CLOCK_ZONE_KEY)
  } catch {
    // Storage can be blocked; the choice then lasts until reload.
  }
  memoryZone = zone
  window.dispatchEvent(new Event(CLOCK_ZONE_EVENT))
}

// The current choice, read from storage once and then kept here, so blocked storage still updates
// the clock for this page load. A change from another tab clears it to be read again.
let memoryZone: string | null = null

function subscribe(onChange: () => void) {
  const onStorage = (e: StorageEvent) => {
    if (e.key === CLOCK_ZONE_KEY) {
      memoryZone = null
      onChange()
    }
  }
  window.addEventListener(CLOCK_ZONE_EVENT, onChange)
  window.addEventListener('storage', onStorage)
  return () => {
    window.removeEventListener(CLOCK_ZONE_EVENT, onChange)
    window.removeEventListener('storage', onStorage)
  }
}

function snapshot() {
  memoryZone ??= readClockZone()
  return memoryZone
}

/** The pinned zone ('' when following the device), shared by every component and tab. */
export function useClockZone(): string {
  return useSyncExternalStore(subscribe, snapshot, () => '')
}

/** "Africa/Kampala" -> "Kampala", "America/Argentina/Buenos_Aires" -> "Buenos Aires". */
export function zoneCity(zone: string): string {
  const last = zone.split('/').pop() ?? zone
  return last.replace(/_/g, ' ')
}

export interface ClockReading {
  /** 24-hour "14:05". */
  time: string
  /** Short zone name such as "EAT" or "GMT+3". */
  abbrev: string
  /** Full line for a tooltip: "Monday, 12 October 2026, 14:05 EAT (Africa/Kampala)". */
  full: string
}

export function readClock(now: Date, zone: string): ClockReading {
  const time = new Intl.DateTimeFormat('en-GB', { timeZone: zone, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(now)
  const abbrev =
    new Intl.DateTimeFormat('en-GB', { timeZone: zone, timeZoneName: 'short' })
      .formatToParts(now)
      .find((p) => p.type === 'timeZoneName')?.value ?? zone
  const day = new Intl.DateTimeFormat('en-GB', { timeZone: zone, weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }).format(now)
  return { time, abbrev, full: `${day}, ${time} ${abbrev} (${zone})` }
}
