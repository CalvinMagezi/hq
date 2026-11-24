import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { isCachedApi } from './swCache'

describe('what the service worker keeps for offline use', () => {
  test('a note and the suite reads of it are kept', () => {
    for (const p of ['/api/note', '/api/suite/sheet', '/api/suite/plot', '/api/suite/blocks', '/api/suite/media', '/api/suite/tile']) expect(isCachedApi(p)).toBe(true)
  })
  test('what changes things, and what is private to a moment, is not', () => {
    for (const p of ['/api/note/export', '/api/suite/sheet/cell', '/api/suite/sheet/rows', '/api/suite/blocks/edit', '/api/suite/shrink', '/api/suite/export', '/api/ws-ticket']) expect(isCachedApi(p)).toBe(false)
  })
  test('the worker only ever caches a GET', () => {
    const sw = readFileSync(new URL('../sw.ts', import.meta.url), 'utf8')
    expect(sw).toContain("request.method !== 'GET'")
    expect(sw).toContain("from './lib/swCache'")
  })
})
