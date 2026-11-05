import { describe, expect, test } from 'bun:test'
import { capQueries, keyedStorage, type KeyValue, noteHqId, PLACE_CAP_BYTES, placeSavedAt, resetForTest, TOTAL_CAP_BYTES } from './savedCopy'

function memory(): KeyValue & { map: Map<string, string> } {
  const map = new Map<string, string>()
  return {
    map,
    get: async (k) => map.get(k),
    set: async (k, v) => void map.set(k, v),
    del: async (k) => void map.delete(k),
  }
}

type Q = Parameters<typeof capQueries>[0][number]
const query = (place: string, n: number, at: number, bytes: number): Q =>
  ({ queryKey: [place, n], queryHash: `${place}${n}`, state: { data: 'x'.repeat(bytes), dataUpdatedAt: at, status: 'success' } }) as unknown as Q

describe('the saved copy', () => {
  test('two HQs on one origin never read each other', async () => {
    const kv = memory()
    const storage = keyedStorage(kv)
    resetForTest('hq-a')
    await storage.setItem('hq-store', 'notes of A')
    resetForTest('hq-b')
    expect(await storage.getItem('hq-store')).toBeNull()
    await storage.setItem('hq-store', 'notes of B')
    resetForTest('hq-a')
    expect(await storage.getItem('hq-store')).toBe('notes of A')
    expect([...kv.map.keys()].sort()).toEqual(['hq-store:hq-a', 'hq-store:hq-b'])
  })

  test('with no HQ id nothing is read or kept', async () => {
    const kv = memory()
    resetForTest(null)
    const storage = keyedStorage(kv)
    await storage.setItem('hq-store', 'x')
    expect(kv.map.size).toBe(0)
    expect(await storage.getItem('hq-store')).toBeNull()
  })

  test('a different HQ answering reloads the page; the same one does not', () => {
    resetForTest('hq-a')
    let reloads = 0
    expect(noteHqId('hq-a', () => reloads++)).toBe(false)
    expect(noteHqId('hq-b', () => reloads++)).toBe(true)
    expect(reloads).toBe(1)
  })

  test('one place never keeps more than its cap, newest first', () => {
    const big = Math.floor(PLACE_CAP_BYTES / 3)
    const kept = capQueries([1, 2, 3, 4, 5].map((n) => query('notes', n, n, big)))
    expect(kept.map((q) => q.queryKey[1]).sort()).toEqual([4, 5])
  })

  test('past the total cap the place saved longest ago goes first', () => {
    const size = PLACE_CAP_BYTES - 1024
    const places = Array.from({ length: Math.ceil(TOTAL_CAP_BYTES / size) + 2 }, (_, i) => query(`p${i}`, 0, 100 + i, size))
    const kept = capQueries(places)
    const total = kept.reduce((s, q) => s + JSON.stringify(q).length, 0)
    expect(total).toBeLessThanOrEqual(TOTAL_CAP_BYTES)
    expect(kept.some((q) => q.queryKey[0] === 'p0')).toBe(false)
    expect(kept.some((q) => q.queryKey[0] === `p${places.length - 1}`)).toBe(true)
  })

  test('each place has its own saved time', () => {
    const qs = [query('notes', 1, 50, 1), query('notes', 2, 90, 1), query('tasks', 1, 70, 1)]
    expect(placeSavedAt(qs, 'notes')).toBe(90)
    expect(placeSavedAt(qs, 'tasks')).toBe(70)
    expect(placeSavedAt(qs, 'chat')).toBeNull()
  })
})
