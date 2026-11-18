// The rules behind the Inbox's desktop layout: which item opens by itself, which requests fold under
// helpers, the one digest row for helper updates, and what waits for later. Pure, so each is tested.
import type { NotificationItem } from './notificationsApi'
import { updateKind } from '../components/notifications/updateKind'

/** The item the card shows when nothing is open: a sign-in first, since only the owner can answer it, then the oldest. */
export function preferredItem(items: NotificationItem[]): NotificationItem | null {
  const waiting = items.filter((n) => n.state === 'pending')
  const signIn = waiting.find((n) => n.metadata.sign_in)
  if (signIn) return signIn
  return [...waiting].sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at))[0] ?? null
}

/** Requests from helpers fold into one quiet row; everything else stays in the list. */
export function splitHelpers(items: NotificationItem[]): { mine: NotificationItem[]; helpers: NotificationItem[] } {
  const helpers = items.filter((n) => updateKind(n) === 'helpers')
  return { mine: items.filter((n) => updateKind(n) !== 'helpers'), helpers }
}

/** How many helper updates make a digest instead of a row each. */
export const DIGEST_MIN = 3

/** One day's updates with the helper ones gathered, when there are enough of them to be a pile. */
export function digestOf(items: NotificationItem[]): { rows: NotificationItem[]; helpers: NotificationItem[] } {
  const helpers = items.filter((n) => updateKind(n) === 'helpers')
  if (helpers.length < DIGEST_MIN) return { rows: items, helpers: [] }
  return { rows: items.filter((n) => updateKind(n) !== 'helpers'), helpers }
}

/** The item after (or before) the current one, staying at the ends; the first when none is current. */
export function neighbour(items: { id: string }[], currentId: string | undefined, step: 1 | -1): string | null {
  if (items.length === 0) return null
  const at = items.findIndex((n) => n.id === currentId)
  if (at < 0) return items[step === 1 ? 0 : items.length - 1].id
  return items[Math.min(items.length - 1, Math.max(0, at + step))].id
}

const LATER_KEY = 'hq-inbox-later'
/** "Later" brings an update back after this long. */
export const LATER_MS = 3 * 60 * 60 * 1000

export type LaterMap = Record<string, number>

/** What waits for later in this browser, by item id and the time it comes back. */
export function readLater(store: Pick<Storage, 'getItem'> = localStorage): LaterMap {
  try {
    const raw: unknown = JSON.parse(store.getItem(LATER_KEY) ?? '{}')
    if (!raw || typeof raw !== 'object') return {}
    return Object.fromEntries(Object.entries(raw).filter((e): e is [string, number] => typeof e[1] === 'number'))
  } catch {
    return {}
  }
}

export function writeLater(map: LaterMap, store: Pick<Storage, 'setItem'> = localStorage): void {
  try {
    store.setItem(LATER_KEY, JSON.stringify(map))
  } catch {
    // Blocked storage: Later lasts until the page is closed.
  }
}

/** Updates that are waiting for later, and the rest. One that came due is back in the list. */
export function splitLater(items: NotificationItem[], later: LaterMap, now: number): { waiting: NotificationItem[]; rest: NotificationItem[] } {
  const waiting = items.filter((n) => (later[n.id] ?? 0) > now)
  return { waiting, rest: items.filter((n) => !((later[n.id] ?? 0) > now)) }
}
