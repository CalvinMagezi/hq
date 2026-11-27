import { afterEach, describe, expect, test } from 'bun:test'
import { isHeldBack, listMemories, memoryBadges, recallModeKey, type MemoryItem } from './memoryApi'

const item = (over: Partial<MemoryItem> = {}): MemoryItem => ({
  id: 1,
  summary: 'Acme delivers on Thursdays',
  entities: [],
  topics: [],
  importance: 0.5,
  created_at: '2026-06-01T00:00:00+00:00',
  status: 'active',
  origin: 'owner',
  provenance: 'model_inferred',
  pinned: false,
  used: 0,
  thread_id: null,
  harness: 'web',
  ...over,
})

describe('what is unusual about a memory', () => {
  test('an ordinary memory has no badges', () => {
    expect(memoryBadges(item())).toEqual([])
    expect(isHeldBack(item())).toBe(false)
  })

  test('each unusual thing is named once, pinned first', () => {
    const odd = item({ pinned: true, status: 'review', origin: 'unknown_channel', provenance: 'owner_edited' })
    expect(memoryBadges(odd)).toEqual([
      'memory_badge_pinned',
      'memory_badge_review',
      'memory_badge_shared',
      'memory_badge_edited',
    ])
    expect(memoryBadges(item({ status: 'superseded' }))).toEqual(['memory_badge_outdated'])
  })

  test('a memory is held back when it is under review or from a shared channel', () => {
    expect(isHeldBack(item({ status: 'review' }))).toBe(true)
    expect(isHeldBack(item({ origin: 'unknown_channel' }))).toBe(true)
    expect(isHeldBack(item({ status: 'superseded' }))).toBe(false)
  })
})

describe('how recall is described', () => {
  test('every mode has a line, and an unknown one has none', () => {
    expect(recallModeKey('shadow')).toBe('memory_mode_shadow')
    expect(recallModeKey('query')).toBe('memory_mode_query')
    expect(recallModeKey('recent')).toBe('memory_mode_recent')
    expect(recallModeKey('off')).toBe('memory_mode_off')
    expect(recallModeKey(null)).toBeNull()
  })
})

describe('the list request', () => {
  const realFetch = globalThis.fetch
  const urls: string[] = []
  afterEach(() => {
    globalThis.fetch = realFetch
    urls.length = 0
  })
  const fake = () => {
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      urls.push(String(input))
      return new Response(JSON.stringify({ counts: {}, items: [], recall: 'shadow' }), { status: 200, headers: { 'content-type': 'application/json' } })
    }) as typeof fetch
  }

  test('a filter or a search is sent, and a search wins over a filter', async () => {
    fake()
    await listMemories('all', '')
    await listMemories('review', '')
    await listMemories('pinned', '  acme feeds ')
    expect(urls.map((u) => u.replace(/^https?:\/\/[^/]+/, ''))).toEqual([
      '/api/memory',
      '/api/memory?filter=review',
      '/api/memory?q=acme+feeds',
    ])
  })
})
