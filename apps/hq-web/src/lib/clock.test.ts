import { expect, test } from 'bun:test'
import { availableTimeZones, isValidTimeZone, readClock, zoneCity } from './clock'

test('isValidTimeZone accepts IANA names and rejects junk', () => {
  expect(isValidTimeZone('Africa/Kampala')).toBe(true)
  expect(isValidTimeZone('UTC')).toBe(true)
  expect(isValidTimeZone('Mars/Olympus_Mons')).toBe(false)
  expect(isValidTimeZone('')).toBe(false)
  expect(isValidTimeZone(42)).toBe(false)
})

test('readClock shows 24-hour time in the chosen zone', () => {
  const at = new Date('2026-10-12T11:05:00Z')
  expect(readClock(at, 'UTC').time).toBe('11:05')
  expect(readClock(at, 'Africa/Kampala').time).toBe('14:05')
  expect(readClock(new Date('2026-10-12T23:30:00Z'), 'Africa/Kampala').time).toBe('02:30')
  const r = readClock(at, 'Africa/Kampala')
  expect(r.full).toContain('Monday')
  expect(r.full).toContain('(Africa/Kampala)')
  expect(r.abbrev.length).toBeGreaterThan(0)
})

test('zoneCity keeps the last segment readable', () => {
  expect(zoneCity('Africa/Kampala')).toBe('Kampala')
  expect(zoneCity('America/Argentina/Buenos_Aires')).toBe('Buenos Aires')
  expect(zoneCity('UTC')).toBe('UTC')
})

test('availableTimeZones is sorted, unique and includes UTC', () => {
  const zones = availableTimeZones()
  expect(zones).toContain('UTC')
  expect(zones).toContain('Africa/Kampala')
  expect(new Set(zones).size).toBe(zones.length)
  expect([...zones].sort()).toEqual(zones)
})

test('the old clock key moves to hq-display-tz on first read', async () => {
  const store = new Map<string, string>([['hq.clock.timeZone', 'Africa/Kampala']])
  ;(globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  }
  const { readClockZone } = await import('./clock')
  expect(readClockZone()).toBe('Africa/Kampala')
  expect(store.get('hq-display-tz')).toBe('Africa/Kampala')
  expect(store.has('hq.clock.timeZone')).toBe(false)
})
