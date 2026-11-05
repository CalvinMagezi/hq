// Version skew: this page against the HQ that answers it (web wave 2, spec 03).
// The page carries its own client level; /api/hello says the lowest level HQ still serves.

import { useSyncExternalStore } from 'react'
import { t } from '~/strings'
import { setUpgradeHandler } from './hqAuth'
import { useToasts } from './toast'

declare const __APP_VERSION__: string

/** The client level this build speaks. HQ's `min_client` above it means the page is too old. */
export const CLIENT_LEVEL = 1
/** This build's own version, injected by the bundler; absent in tests. */
export const APP_VERSION: string = typeof __APP_VERSION__ === 'string' ? __APP_VERSION__ : 'dev'
export const HTTP_UPGRADE_REQUIRED = 426

/** Whether HQ no longer serves a page of `level`. */
export function pageIsOlder(minClient: unknown, level: number = CLIENT_LEVEL): boolean {
  return typeof minClient === 'number' && minClient > level
}

interface Skew {
  /** HQ's hello said this page is too old. */
  hello: boolean
  /** A newer shell waits in the service worker. */
  waiting: boolean
  /** HQ refused a call from this page with 426. */
  refused: boolean
}

let skew: Skew = { hello: false, waiting: false, refused: false }
const listeners = new Set<() => void>()
let toastId: number | null = null

function set(next: Partial<Skew>) {
  const merged = { ...skew, ...next }
  if (merged.hello === skew.hello && merged.waiting === skew.waiting && merged.refused === skew.refused) return
  skew = merged
  for (const l of listeners) l()
  if (isNewer() && !(toastId !== null && useToasts.getState().toasts.some((x) => x.id === toastId))) showNewerToast()
}

export function isNewer(s: Skew = skew): boolean {
  return s.hello || s.waiting || s.refused
}

/** V1: one toast that stays until used or closed, with Reload. */
function showNewerToast() {
  toastId = useToasts.getState().push({ text: t('newer_hq'), kind: 'status', action: { label: t('card_newer_act'), run: () => void reloadForNewer() } })
}

/** Called with every hello the page reads. */
export function noteHelloVersion(hello: { min_client?: number } | null) {
  if (hello) set({ hello: pageIsOlder(hello.min_client) })
}

export function noteWaitingWorker(waiting: boolean) {
  set({ waiting })
}

/** V4: HQ refused a call for an old page. */
export function noteRefused() {
  set({ refused: true })
}

setUpgradeHandler(noteRefused)

/** True when HQ is newer than this page, by hello, by a waiting worker, or by a refused call. */
export function useNewerHq(): boolean {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb)
      return () => void listeners.delete(cb)
    },
    isNewer,
    () => false,
  )
}

const flushers = new Set<() => void>()

/** A place that holds unsent text registers a function that writes it down before a reload. */
export function onBeforeReload(flush: () => void): () => void {
  flushers.add(flush)
  return () => void flushers.delete(flush)
}

/** Writes every draft down, hands the waiting shell the turn, and reloads. The outbox lives in IndexedDB and survives. */
export async function reloadForNewer(reload: () => void = () => window.location.reload()) {
  for (const flush of flushers) {
    try {
      flush()
    } catch {
      // A draft that cannot be written must not stop the reload.
    }
  }
  try {
    if ('serviceWorker' in navigator) {
      const reg = await navigator.serviceWorker.getRegistration()
      reg?.waiting?.postMessage({ type: 'SKIP_WAITING' })
    }
  } catch {
    // No worker: the plain reload still fetches the new page.
  }
  reload()
}

const UNSAVED_PREFIX = 'hq.unsaved:'

/** Keeps text that is not saved anywhere else across one reload (this tab only). */
export function keepUnsaved(key: string, text: string) {
  try {
    sessionStorage.setItem(UNSAVED_PREFIX + key, text)
  } catch {
    // Storage can be blocked; the reload then drops it.
  }
}

/** The text kept for `key` before a reload, once. */
export function takeUnsaved(key: string): string | null {
  try {
    const text = sessionStorage.getItem(UNSAVED_PREFIX + key)
    sessionStorage.removeItem(UNSAVED_PREFIX + key)
    return text
  } catch {
    return null
  }
}

/** The failure line for an action: the "newer HQ" form when HQ refused with 426, else `fallback`. */
export function failureLine(err: unknown, newerKey: Parameters<typeof t>[0], fallback: string): string {
  return (err as { status?: number } | null)?.status === HTTP_UPGRADE_REQUIRED ? t(newerKey) : fallback
}

export function resetVersionForTests() {
  skew = { hello: false, waiting: false, refused: false }
  toastId = null
  flushers.clear()
}
