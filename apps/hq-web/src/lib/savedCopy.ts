// The saved copy, one per HQ (docs/plans/ux-run/web/wave-0/02-shared-parts.md rules 7 to 9). One origin
// can serve two HQs (the HQ window shows this computer's and a phone's), so every store is named with
// the HQ's install id from /api/hello and a different id never reads another's copy.

import type { PersistedClient } from '@tanstack/react-query-persist-client'
import { isSharedComputer } from './sharedComputer'

const LAST_ID_KEY = 'hq-last-id'
/** How long a first visit waits for /api/hello before it starts with no saved copy. */
export const HELLO_WAIT_MS = 3000
/** The cap a place's copy keeps, and the whole copy's (resolutions.md B16, the phone's too). */
export const PLACE_CAP_BYTES = 256 * 1024
export const TOTAL_CAP_BYTES = 2 * 1024 * 1024
/** Store names before they were keyed by HQ id; dropped the first time an id is known. */
export const LEGACY_KEYS = ['hq-query-cache', 'hq-store', 'hq-thread-store']

export const keyFor = (name: string, hqId: string) => `${name}:${hqId}`

function readLastId(): string | null {
  try {
    return localStorage.getItem(LAST_ID_KEY)
  } catch {
    return null
  }
}

// The id this page last knew. It is trusted only once this page load's own hello agrees, or after
// HELLO_WAIT_MS with no answer (offline), so a page now fronting another HQ never reads the old copy.
const stored: string | null = typeof window === 'undefined' ? null : readLastId()
let current: string | null = null
let resolveReady: (id: string | null) => void = () => {}
let ready: Promise<string | null> = new Promise((resolve) => {
  resolveReady = resolve
  if (typeof window === 'undefined') return resolve(null)
  setTimeout(() => {
    // No answer in time: start offline on the copy this browser last kept.
    if (current === null) current = stored
    tellWorkerHqId()
    resolve(current)
  }, HELLO_WAIT_MS)
  // Imported when needed, since hello reports back here.
  void import('./hello').then((m) => m.fetchHello())
})

/** The HQ this page's stores belong to, or null until /api/hello has said. */
export function currentHqId(): string | null {
  return current
}

/** Resolves once the stores know which HQ they belong to (or the first visit gave up waiting). */
export function hqIdReady(): Promise<string | null> {
  return ready
}

/** The worker names its API cache with the page's HQ id, so the page tells it. */
export function tellWorkerHqId() {
  // On a shared computer the worker never learns the id, so it keeps no API copy.
  if (!current || isSharedComputer() || typeof navigator === 'undefined' || !navigator.serviceWorker) return
  navigator.serviceWorker.controller?.postMessage({ type: 'hq-id', id: current })
}

if (typeof window !== 'undefined' && typeof navigator !== 'undefined' && navigator.serviceWorker) {
  tellWorkerHqId()
  navigator.serviceWorker.addEventListener('controllerchange', tellWorkerHqId)
}

/**
 * Records which HQ answered. A different HQ than the one this page loaded for reloads the page, so
 * no store keeps the other HQ's data in memory. Returns whether the page is reloading.
 */
export function noteHqId(id: string, reload: () => void = () => window.location.reload()): boolean {
  if (current === id) return false
  // The page's first hello: it settles which copy this page uses, the stored id agreeing or not.
  const first = current === null
  try {
    localStorage.setItem(LAST_ID_KEY, id)
  } catch {
    // Blocked storage: this tab still keys by the id.
  }
  if (!first) {
    reload()
    return true
  }
  current = id
  ready = Promise.resolve(id)
  resolveReady(id)
  tellWorkerHqId()
  return false
}

/** Test seam: start over as a page that has seen `id` (or none). */
export function resetForTest(id: string | null) {
  current = id
  ready = Promise.resolve(id)
}

/** What the saved copy is written to: IndexedDB in the browser, a map in tests. */
export interface KeyValue {
  get(key: string): Promise<string | undefined>
  set(key: string, value: string): Promise<void>
  del(key: string): Promise<void>
}

/** A storage whose keys carry the current HQ id. Reads wait for the id; with none, nothing is read or kept. */
export function keyedStorage(kv: KeyValue) {
  return {
    getItem: async (name: string): Promise<string | null> => {
      if (isSharedComputer()) return null
      const id = await hqIdReady()
      return id ? ((await kv.get(keyFor(name, id))) ?? null) : null
    },
    setItem: async (name: string, value: string): Promise<void> => {
      // A shared computer keeps nothing from HQ.
      if (isSharedComputer()) return
      const id = currentHqId()
      if (id) await kv.set(keyFor(name, id), value)
    },
    removeItem: async (name: string): Promise<void> => {
      const id = currentHqId()
      if (id) await kv.del(keyFor(name, id))
    },
  }
}

type DehydratedQuery = PersistedClient['clientState']['queries'][number]

/** A place is the first part of a query key: `['notes', path]` belongs to `notes`. */
export function placeOf(query: DehydratedQuery): string {
  const first = query.queryKey[0]
  return typeof first === 'string' ? first : JSON.stringify(first)
}

const sizeOf = (q: DehydratedQuery) => JSON.stringify(q).length

/**
 * Keeps each place under PLACE_CAP_BYTES (its newest items first) and the whole copy under
 * TOTAL_CAP_BYTES, dropping the place saved longest ago first.
 */
export function capQueries(queries: DehydratedQuery[]): DehydratedQuery[] {
  const byPlace = new Map<string, DehydratedQuery[]>()
  for (const q of queries) byPlace.set(placeOf(q), [...(byPlace.get(placeOf(q)) ?? []), q])
  const places: { kept: DehydratedQuery[]; bytes: number; savedAt: number }[] = []
  for (const list of byPlace.values()) {
    const newestFirst = [...list].sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt)
    const kept: DehydratedQuery[] = []
    let bytes = 0
    for (const q of newestFirst) {
      const size = sizeOf(q)
      if (bytes + size > PLACE_CAP_BYTES) continue
      kept.push(q)
      bytes += size
    }
    places.push({ kept, bytes, savedAt: newestFirst[0]?.state.dataUpdatedAt ?? 0 })
  }
  places.sort((a, b) => a.savedAt - b.savedAt)
  let total = places.reduce((sum, p) => sum + p.bytes, 0)
  while (total > TOTAL_CAP_BYTES && places.length > 0) total -= places.shift()!.bytes
  return places.flatMap((p) => p.kept)
}

/** When a place's copy was last brought up to date by HQ, from the queries it holds. */
export function placeSavedAt(queries: { queryKey: readonly unknown[]; state: { dataUpdatedAt: number } }[], place: string): number | null {
  const times = queries.filter((q) => q.queryKey[0] === place).map((q) => q.state.dataUpdatedAt).filter((t) => t > 0)
  return times.length ? Math.max(...times) : null
}
