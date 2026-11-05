import { beforeEach, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DataUsedPage } from '~/components/settings/BrowserSections'
import { addBytes, lastHourBytes, overIdleTarget, searchDebounceMs, sumTransfer } from './dataUsage'
import { readDataUsed } from './dataUsed'

const store = new Map<string, string>()
beforeEach(() => {
  store.clear()
  ;(globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  }
})

const at = (iso: string) => new Date(iso)

test('bytes count against the hour and the month, and an earlier hour reads as the idle hour', () => {
  addBytes(300_000, at('2026-11-05T10:05:00Z'))
  addBytes(100_000, at('2026-11-05T10:50:00Z'))
  addBytes(5_000, at('2026-11-05T11:01:00Z'))
  const now = at('2026-11-05T11:30:00Z')
  expect(lastHourBytes(now)).toBe(400_000)
  expect(readDataUsed(now)).toBe(405_000)
})

test('no whole hour yet means no idle number', () => {
  addBytes(1_000, at('2026-11-05T10:05:00Z'))
  expect(lastHourBytes(at('2026-11-05T10:30:00Z'))).toBeNull()
})

test('the monthly counter starts over on the 1st', () => {
  addBytes(9_000, at('2026-10-31T10:00:00Z'))
  expect(readDataUsed(at('2026-11-02T10:00:00Z'))).toBe(0)
})

test('only HQ-origin transfers are summed, and cached ones count as zero', () => {
  const entries = [
    { name: 'https://hq.example.ts.net/assets/a.js', transferSize: 1000 },
    { name: 'https://hq.example.ts.net/api/x', transferSize: 0 },
    { name: 'https://other.example.com/x.js', transferSize: 5000 },
    { name: 'https://hq.example.ts.net/assets/b.css' },
  ]
  expect(sumTransfer(entries, 'https://hq.example.ts.net')).toBe(1000)
})

test('over 1 MB an idle hour is over the target', () => {
  expect(overIdleTarget(1_600_000)).toBe(true)
  expect(overIdleTarget(400_000)).toBe(false)
})

test('data saver makes Search wait 300 ms after the last key', () => {
  expect(searchDebounceMs(200, true)).toBe(300)
  expect(searchDebounceMs(200, false)).toBe(200)
})

test('check 5: Data used shows three rows, the first "Data used by HQ this month"', () => {
  const html = renderToStaticMarkup(createElement(DataUsedPage))
  const titles = [...html.matchAll(/<b>([^<]*)<\/b>/g)].map((m) => m[1])
  expect(titles).toEqual(['Data used by HQ this month', 'An hour with HQ open and idle', 'Opening Chat'])
})
