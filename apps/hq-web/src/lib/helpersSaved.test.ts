import { expect, test } from 'bun:test'
import { helpersAway, readSavedHelpers, saveHelpers } from './helpersSaved'
import { resetForTest } from './savedCopy'

const memory = () => {
  const m = new Map<string, string>()
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m }
}

test('away covers offline and unreachable; starting is its own state', () => {
  expect(helpersAway('offline')).toBe('away')
  expect(helpersAway('unreachable')).toBe('away')
  expect(helpersAway('starting')).toBe('starting')
  expect(helpersAway('connected')).toBeNull()
})

test('a saved helpers list is named by HQ id and read back with its time', () => {
  resetForTest('hq-a')
  const store = memory()
  saveHelpers([{ id: 's1' } as never], 1234, store)
  expect([...store.m.keys()]).toEqual(['hq-helpers-saved:hq-a'])
  expect(readSavedHelpers(store)).toEqual({ at: 1234, list: [{ id: 's1' }] })
})

test('nothing is saved without an HQ id', () => {
  resetForTest(null)
  const store = memory()
  saveHelpers([], 1, store)
  expect(store.m.size).toBe(0)
  expect(readSavedHelpers(store)).toBeNull()
})
