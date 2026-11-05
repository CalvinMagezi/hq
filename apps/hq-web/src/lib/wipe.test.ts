import { describe, expect, test } from 'bun:test'
import { API_CACHE, WIPED_IDB_KEYS, WIPED_LOCAL_KEYS, type WipeDeps, wipeWith } from './wipe'

function fakeBrowser() {
  const idb = new Map<string, string>(WIPED_IDB_KEYS.map((k) => [k, 'saved']))
  idb.set('hq-other', 'kept')
  const local = new Map<string, string>(WIPED_LOCAL_KEYS.map((k) => [k, 'saved']))
  local.set('hq-install-dismissed', '1')
  local.set('hq.clock.timeZone', 'Africa/Kampala')
  const caches = new Set([API_CACHE, 'hq-static'])
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
    await wipeWith(b.deps)
    for (const k of WIPED_IDB_KEYS) expect(b.idb.has(k)).toBe(false)
    for (const k of WIPED_LOCAL_KEYS) expect(b.local.has(k)).toBe(false)
    expect(b.caches.has(API_CACHE)).toBe(false)
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
      if (k === 'hq.chat.drafts') throw new Error('blocked')
      b.local.delete(k)
    }
    await wipeWith(b.deps)
    expect(b.local.has('hq-last-sync')).toBe(false)
    for (const k of WIPED_IDB_KEYS) expect(b.idb.has(k)).toBe(false)
  })

  test('a wipe heard from another tab is not announced again', async () => {
    const b = fakeBrowser()
    await wipeWith(b.deps, { announce: false })
    expect(b.told).toEqual(['memory', 'worker'])
  })
})
