// Wiping the browser when HQ refuses its sign-in (docs/plans/ux-run/web/wave-0/01-live-bugs.md
// rules 9 to 11): every copy of HQ's data goes at once, so a signed-out laptop shows nothing.
// Per-browser conveniences that hold no HQ content (the install prompt, the clock zone, layout)
// stay.

import { del } from 'idb-keyval'
import { queryClient } from './offlineCache'
import { useHQStore } from '~/store/hqStore'
import { useThreadStore } from '~/store/threadStore'
import { clearDrafts } from '~/components/chat/draftStore'
import { currentHqId, keyFor, LEGACY_KEYS } from './savedCopy'

/** One HQ's stores, each named `{name}:{hq_id}` (shared parts rule 7). */
export const WIPED_IDB_NAMES = ['hq-query-cache', 'hq-store', 'hq-thread-store', 'hq-outbox']
// The token itself is dropped by hqAuth before the wipe starts (see refuseSignIn).
export const WIPED_LOCAL_NAMES = ['hq-drafts', 'hq-last-sync']
export const API_CACHE = 'hq-api'
/** Names from before the stores were keyed by HQ id; always cleared too. */
const LEGACY_LOCAL_KEYS = ['hq.chat.drafts', 'hq-last-sync']

/** Every key a wipe removes for `hqId`, the old unkeyed ones included. */
export function wipedKeys(hqId: string | null) {
  const keyed = (names: string[]) => (hqId ? names.map((n) => keyFor(n, hqId)) : [])
  return {
    idb: [...keyed(WIPED_IDB_NAMES), ...LEGACY_KEYS],
    local: [...keyed(WIPED_LOCAL_NAMES), ...LEGACY_LOCAL_KEYS],
    caches: [...keyed([API_CACHE]), API_CACHE],
  }
}
export const AUTH_CHANNEL = 'hq-auth'
export const SIGNED_OUT_MESSAGE = { type: 'signed-out' } as const

/** What a wipe touches, injected so `bun test` can run it without a browser. */
export interface WipeDeps {
  resetMemory: () => void
  removeIdb: (key: string) => Promise<void>
  removeLocal: (key: string) => void
  deleteCache: (name: string) => Promise<unknown>
  unsubscribePush: () => Promise<unknown>
  tellWorker: () => void
  tellTabs: () => void
}

/** Clears every store; one failing store never stops the others. */
export async function wipeWith(
  deps: WipeDeps,
  { announce = true, hqId = currentHqId() }: { announce?: boolean; hqId?: string | null } = {},
): Promise<void> {
  const keys = wipedKeys(hqId)
  // What is on screen goes first, before the next paint.
  deps.resetMemory()
  for (const key of keys.local) {
    try {
      deps.removeLocal(key)
    } catch {
      // Blocked storage holds nothing to wipe.
    }
  }
  if (announce) deps.tellTabs()
  deps.tellWorker()
  await Promise.allSettled([
    ...keys.idb.map((k) => deps.removeIdb(k)),
    ...keys.caches.map((c) => deps.deleteCache(c)),
    deps.unsubscribePush(),
  ])
}

function resetMemory() {
  queryClient.clear()
  useHQStore.setState(useHQStore.getInitialState(), true)
  useThreadStore.setState(useThreadStore.getInitialState(), true)
  clearDrafts()
}

async function unsubscribePush() {
  const reg = await navigator.serviceWorker?.getRegistration()
  const sub = await reg?.pushManager.getSubscription()
  await sub?.unsubscribe()
}

function browserDeps(): WipeDeps {
  return {
    resetMemory,
    removeIdb: (key) => (typeof indexedDB === 'undefined' ? Promise.resolve() : del(key)),
    removeLocal: (key) => localStorage.removeItem(key),
    deleteCache: (name) => (typeof caches === 'undefined' ? Promise.resolve() : caches.delete(name)),
    unsubscribePush,
    tellWorker: () => navigator.serviceWorker?.controller?.postMessage(SIGNED_OUT_MESSAGE),
    tellTabs: () => {
      if (typeof BroadcastChannel === 'undefined') return
      const channel = new BroadcastChannel(AUTH_CHANNEL)
      channel.postMessage(SIGNED_OUT_MESSAGE)
      channel.close()
    },
  }
}

/** Wipes this browser of HQ's data, and tells the worker and the other tabs. */
export function wipeBrowser(): Promise<void> {
  return wipeWith(browserDeps())
}

/** Another tab of this HQ was refused: wipe here too, without echoing it back. `onSignedOut` says
 * whether this tab was still signed in; one already signed out has nothing left to wipe. */
export function listenForWipe(onSignedOut: () => boolean): () => void {
  if (typeof BroadcastChannel === 'undefined') return () => {}
  const channel = new BroadcastChannel(AUTH_CHANNEL)
  channel.onmessage = (event) => {
    if (event.data?.type !== SIGNED_OUT_MESSAGE.type) return
    // The screen goes first, before the next paint.
    if (onSignedOut()) void wipeWith(browserDeps(), { announce: false })
  }
  return () => channel.close()
}
