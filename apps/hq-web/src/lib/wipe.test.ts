import { describe, expect, test } from 'bun:test'
import { type WipeDeps, wipedKeys, wipeWith } from './wipe'

const A = 'hq-a'
const keysA = wipedKeys(A)

function fakeBrowser() {
  const idb = new Map<string, string>(keysA.idb.map((k) => [k, 'saved']))
  idb.set('hq-other', 'kept')
  // Another HQ on the same origin keeps its copy.
  idb.set('hq-store:hq-b', 'kept')
  const local = new Map<string, string>(keysA.local.map((k) => [k, 'saved']))
  local.set('hq-install-dismissed', '1')
  local.set('hq.clock.timeZone', 'Africa/Kampala')
  const caches = new Set([...keysA.caches, 'hq-api:hq-b', 'hq-static'])
  const told: string[] = []
  let push = true
  const deps: WipeDeps = {
    resetMemory: () => told.push('memory'),
    removeIdb: async (k) => void idb.delete(k),
    removeLocal: (k) => void local.delete(k),
    deleteCache: async (n) => caches.delete(n),
    unsubscribePush: async () => {
      push = false
    },
    tellWorker: () => told.push('worker'),
    tellTabs: () => told.push('tabs'),
  }
  return { deps, idb, local, caches, told, push: () => push }
}

describe('wipeWith', () => {
  test('clears every HQ store and keeps per-browser conveniences', async () => {
    const b = fakeBrowser()
    await wipeWith(b.deps, { hqId: A })
    for (const k of keysA.idb) expect(b.idb.has(k)).toBe(false)
    for (const k of keysA.local) expect(b.local.has(k)).toBe(false)
    for (const c of keysA.caches) expect(b.caches.has(c)).toBe(false)
    expect(b.idb.get('hq-store:hq-b')).toBe('kept')
    expect(b.caches.has('hq-api:hq-b')).toBe(true)
    expect(b.push()).toBe(false)
    expect(b.told).toEqual(['memory', 'tabs', 'worker'])
    expect(b.local.get('hq-install-dismissed')).toBe('1')
    expect(b.local.get('hq.clock.timeZone')).toBe('Africa/Kampala')
    expect(b.caches.has('hq-static')).toBe(true)
  })

  test('a store that fails does not stop the rest', async () => {
    const b = fakeBrowser()
    b.deps.deleteCache = async () => {
      throw new Error('blocked')
    }
    b.deps.removeLocal = (k) => {
      if (k === 'hq-drafts:hq-a') throw new Error('blocked')
      b.local.delete(k)
    }
    await wipeWith(b.deps, { hqId: A })
    expect(b.local.has('hq-last-sync:hq-a')).toBe(false)
    for (const k of keysA.idb) expect(b.idb.has(k)).toBe(false)
  })

  test('a wipe heard from another tab is not announced again', async () => {
    const b = fakeBrowser()
    await wipeWith(b.deps, { announce: false, hqId: A })
    expect(b.told).toEqual(['memory', 'worker'])
  })
})
