// Data saved on the device so every screen paints from the last good copy and
// refreshes behind it. IndexedDB, not localStorage: no 5 MB cap for notes and
// chat history.
import { QueryClient } from '@tanstack/react-query'
import { createAsyncStoragePersister } from '@tanstack/query-async-storage-persister'
import { del, get, set } from 'idb-keyval'
import type { StateStorage } from 'zustand/middleware'

const DAY_MS = 24 * 60 * 60 * 1000
export const CACHE_MAX_AGE_MS = 7 * DAY_MS
// Bump when a cached response shape changes, so old saved data is discarded.
export const CACHE_BUSTER = 'v1'
const LAST_SYNC_KEY = 'hq-last-sync'

// The build prerenders the shell without a browser, where IndexedDB is absent.
const hasIdb = () => typeof indexedDB !== 'undefined'

const idbStore = {
  getItem: async (key: string): Promise<string | null> => (hasIdb() ? ((await get<string>(key)) ?? null) : null),
  setItem: async (key: string, value: string): Promise<void> => {
    if (hasIdb()) await set(key, value)
  },
  removeItem: async (key: string): Promise<void> => {
    if (hasIdb()) await del(key)
  },
}

/** zustand `persist` storage backed by IndexedDB. */
export const idbStateStorage: StateStorage = idbStore

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

export const queryPersister = createAsyncStoragePersister({
  storage: idbStore,
  key: 'hq-query-cache',
  throttleTime: 1000,
})

/** Called on every successful API response; drives the "saved data" marker. */
export function markSynced() {
  try {
    localStorage.setItem(LAST_SYNC_KEY, String(Date.now()))
  } catch {
    // Private mode or blocked storage: the marker just stays unknown.
  }
}

export function lastSyncedAt(): number | null {
  try {
    const raw = localStorage.getItem(LAST_SYNC_KEY)
    return raw ? Number(raw) : null
  } catch {
    return null
  }
}
