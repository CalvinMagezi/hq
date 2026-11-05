// The helpers list kept for when HQ can't be reached (docs/plans/ux-run/web/wave-4/07-helpers.md,
// "Saved helpers"). It is named with the HQ's id like every saved copy and kept nowhere on a shared computer.

import type { ConnectionState } from './connection'
import { currentHqId, keyFor, PLACE_CAP_BYTES } from './savedCopy'
import { isSharedComputer } from './sharedComputer'
import type { HarnessSession } from './sessionsApi'

export const HELPERS_SAVED_NAME = 'hq-helpers-saved'

export interface SavedHelpers {
  at: number
  list: HarnessSession[]
}

/** Away means the saved list is shown and nothing is sent; Starting says why in its own words. */
export type HelpersAway = 'away' | 'starting' | null

export function helpersAway(state: ConnectionState): HelpersAway {
  if (state === 'starting') return 'starting'
  return state === 'offline' || state === 'unreachable' ? 'away' : null
}

export function saveHelpers(list: HarnessSession[], now: number = Date.now(), storage: Pick<Storage, 'setItem'> | null = browserStorage()) {
  const id = currentHqId()
  if (!storage || !id || isSharedComputer()) return
  const body = JSON.stringify({ at: now, list } satisfies SavedHelpers)
  if (body.length > PLACE_CAP_BYTES) return
  try {
    storage.setItem(keyFor(HELPERS_SAVED_NAME, id), body)
  } catch {
    // Full or blocked storage: the list is simply not kept.
  }
}

export function readSavedHelpers(storage: Pick<Storage, 'getItem'> | null = browserStorage()): SavedHelpers | null {
  const id = currentHqId()
  if (!storage || !id || isSharedComputer()) return null
  try {
    const raw = storage.getItem(keyFor(HELPERS_SAVED_NAME, id))
    const parsed = raw ? (JSON.parse(raw) as Partial<SavedHelpers>) : null
    return parsed && typeof parsed.at === 'number' && Array.isArray(parsed.list) ? { at: parsed.at, list: parsed.list } : null
  } catch {
    return null
  }
}

function browserStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}
