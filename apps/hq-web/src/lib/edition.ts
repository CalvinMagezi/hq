// Which edition of HQ answers (`edition` in /api/hello), and the places that edition serves
// (web wave 2 spec 02). The last answer is kept so the shell draws the right places before hello
// answers; on a first visit nothing is known and the shell draws what is safe on both.

import { useSyncExternalStore } from 'react'
import { PLACES, type Edition, type Place } from './routes'

const STORAGE_KEY = 'hq-edition'
const EDITIONS: readonly string[] = ['full', 'lite', 'mobile']

/** Anything /api/hello says that is not a known edition counts as the full HQ. */
export const toEdition = (raw: unknown): Edition => (raw === 'lite' || raw === 'mobile' ? raw : 'full')

function readStored(): Edition | null {
  if (typeof window === 'undefined') return null
  try {
    const v = window.localStorage.getItem(STORAGE_KEY)
    return v && EDITIONS.includes(v) ? (v as Edition) : null
  } catch {
    return null
  }
}

let current: Edition | null = null
let loaded = false
const listeners = new Set<() => void>()

/** The edition last known in this browser, or null on a first visit. */
export function lastKnownEdition(): Edition | null {
  if (!loaded && typeof window !== 'undefined') {
    current = readStored()
    loaded = true
  }
  return current
}

/** Record what /api/hello said. */
export function setEdition(raw: unknown): void {
  const next = toEdition(raw)
  if (lastKnownEdition() === next) return
  current = next
  try {
    window.localStorage.setItem(STORAGE_KEY, next)
  } catch {
    // not remembered: the next load waits for hello
  }
  for (const l of listeners) l()
}

const subscribe = (l: () => void) => {
  listeners.add(l)
  return () => listeners.delete(l)
}

/** The edition for the shell; null until known. The server render says null so hydration matches. */
export function useEdition(): Edition | null {
  return useSyncExternalStore(subscribe, lastKnownEdition, () => null)
}

/** The places the rail and the bar show. Lite, and an edition not yet known, show Notes, Tasks and Inbox only. */
export function placesFor(edition: Edition | null): Place[] {
  const shown = edition === 'full' || edition === 'mobile' ? PLACES : PLACES.filter((p) => p.id === 'notes' || p.id === 'tasks' || p.id === 'inbox')
  return shown
}
