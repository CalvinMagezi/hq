// Data saved on the device so every screen paints from the last good copy and refreshes behind it.
// IndexedDB, not localStorage: no 5 MB cap for notes and chat history. Every key carries the HQ's
// id (lib/savedCopy.ts), and the copy is capped per place and in all.
import { QueryClient } from '@tanstack/react-query'
import { createAsyncStoragePersister } from '@tanstack/query-async-storage-persister'
import type { PersistedClient } from '@tanstack/react-query-persist-client'
import { del, get, set } from 'idb-keyval'
import type { StateStorage } from 'zustand/middleware'
import { capQueries, currentHqId, hqIdReady, keyedStorage, keyFor, type KeyValue, LEGACY_KEYS } from './savedCopy'

/** The copy is kept until replaced, with its age always shown (shared parts rule 8). */
export const CACHE_MAX_AGE_MS = Number.POSITIVE_INFINITY
// Bump when a cached response shape changes, so old saved data is discarded.
export const CACHE_BUSTER = 'v1'
export const QUERY_CACHE_KEY = 'hq-query-cache'
export const LAST_SYNC_KEY = 'hq-last-sync'

// The build prerenders the shell without a browser, where IndexedDB is absent.
const hasIdb = () => typeof indexedDB !== 'undefined'

export const idb: KeyValue = {
  get: async (key) => (hasIdb() ? get<string>(key) : undefined),
  set: async (key, value) => {
    if (hasIdb()) await set(key, value)
  },
  del: async (key) => {
    if (hasIdb()) await del(key)
  },
}

const keyed = keyedStorage(idb)

/** zustand `persist` storage backed by IndexedDB, one copy per HQ. */
export const idbStateStorage: StateStorage = keyed

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      gcTime: CACHE_MAX_AGE_MS,
      staleTime: 30_000,
      retry: 1,
      refetchOnWindowFocus: true,
      // Show the saved copy even with no connection; fetch whenever one exists.
      networkMode: 'offlineFirst',
    },
  },
})

/** The persisted client with each place and the whole copy held to their caps. */
export function capPersisted(client: PersistedClient): string {
  const queries = capQueries(client.clientState.queries)
  return JSON.stringify({ ...client, clientState: { ...client.clientState, queries } })
}

export const queryPersister = createAsyncStoragePersister({
  storage: keyed,
  key: QUERY_CACHE_KEY,
  throttleTime: 1000,
  serialize: capPersisted,
})

/** Called on every successful API response; drives the "saved data" marker. */
export function markSynced() {
  const id = currentHqId()
  if (!id) return
  try {
    localStorage.setItem(keyFor(LAST_SYNC_KEY, id), String(Date.now()))
  } catch {
    // Private mode or blocked storage: the marker just stays unknown.
  }
}

export function lastSyncedAt(): number | null {
  const id = currentHqId()
  if (!id) return null
  try {
    const raw = localStorage.getItem(keyFor(LAST_SYNC_KEY, id))
    return raw ? Number(raw) : null
  } catch {
    return null
  }
}

/** Unkeyed copies from before the stores were named by HQ id: whose they were is unknown, so they go. */
const LEGACY_LOCAL_KEYS = ['hq.chat.drafts', LAST_SYNC_KEY]

void hqIdReady().then((id) => {
  if (!id) return
  for (const key of LEGACY_KEYS) void idb.del(key)
  try {
    for (const key of LEGACY_LOCAL_KEYS) localStorage.removeItem(key)
  } catch {
    // Blocked storage holds nothing old.
  }
})
