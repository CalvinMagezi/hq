import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { count, read, record } from './noticed'

const g = globalThis as unknown as { localStorage?: Storage }
const original = g.localStorage

beforeEach(() => {
  const m = new Map<string, string>()
  g.localStorage = {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
  } as Storage
})
afterEach(() => {
  g.localStorage = original
})

describe('noticed', () => {
  test('counts add up', () => {
    count('undo.chat_archived', '1.0')
    count('undo.chat_archived', '1.0')
    record('first_load_ms', '1.0', 1200)
    const n = read('1.0')
    expect(n.counts['undo.chat_archived']).toBe(2)
    expect(n.counts.first_load_ms).toBe(1200)
  })

  test('a new version moves the numbers to previous', () => {
    count('edit.within_10s', '1.0', 3)
    count('edit.within_10s', '1.1')
    const n = read('1.1')
    expect(n.counts['edit.within_10s']).toBe(1)
    expect(n.previous).toEqual({ version: '1.0', counts: { 'edit.within_10s': 3 } })
  })

  test('blocked storage never throws', () => {
    g.localStorage = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('blocked')
      },
    } as unknown as Storage
    expect(() => count('outbox.repeats_dropped', '1.0')).not.toThrow()
    expect(read('1.0').counts).toEqual({})
    g.localStorage = undefined
    expect(() => count('approvals.chat', '1.0')).not.toThrow()
  })
})
