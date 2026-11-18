// The catch-up card: when more than five requests wait, most are often from helpers that have since finished
// or stopped. Those can be cleared in one move (cleared, not allowed), and what is left is what needs the person.
import type { NotificationItem } from './notificationsApi'
import type { HarnessSession } from './sessionsApi'
import { lapsed } from './inboxCard'

/** The card appears past this many waiting requests. */
export const PILE_MIN = 5

/** The agent dialog an item is about, if it is the notice of an open one. */
const openDialog = (n: NotificationItem) => (n.metadata.dialog && !n.metadata.dialog.answered ? n.metadata.dialog : null)

/**
 * The waiting requests that are safe to clear in bulk: the notice of an agent's open dialog whose helper is no longer
 * running (finished, stopped, or gone), or that has waited past a day. A sign-in, an approval or a question for the
 * person themselves is never in this list.
 */
export function staleRequests(items: NotificationItem[], sessions: Pick<HarnessSession, 'id' | 'status'>[], now = Date.now()): NotificationItem[] {
  const running = new Set(sessions.filter((s) => s.status === 'running').map((s) => s.id))
  return items.filter((n) => {
    if (n.state !== 'pending' || n.kind !== 'action_needed') return false
    const dialog = openDialog(n)
    if (!dialog) return false
    return !running.has(dialog.session_id) || lapsed(n, now)
  })
}

export interface Pile {
  waiting: number
  old: number
  /** What is left once the old ones are cleared. */
  rest: number
}

/** The card's numbers, or null when there is no pile or nothing safe to clear. */
export function pileOf(waiting: NotificationItem[], stale: NotificationItem[]): Pile | null {
  const count = waiting.filter((n) => n.state === 'pending').length
  if (count <= PILE_MIN || stale.length === 0) return null
  return { waiting: count, old: stale.length, rest: count - stale.length }
}

/** Items cleared from the Inbox that are still an agent's unanswered dialog: what Done lists to restore. */
export function clearedDialogs(items: NotificationItem[]): NotificationItem[] {
  return items.filter((n) => n.state === 'dismissed' && n.kind === 'action_needed' && openDialog(n) !== null)
}

/** Runs `job` over `items` with at most `limit` at once; returns the items it succeeded for, in order. */
export async function inBatches<T>(items: T[], limit: number, job: (item: T) => Promise<unknown>): Promise<T[]> {
  const done: boolean[] = new Array(items.length).fill(false)
  let next = 0
  const worker = async () => {
    for (let i = next++; i < items.length; i = next++) {
      try {
        await job(items[i])
        done[i] = true
      } catch {
        // One that fails stays where it is; the rest go on.
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker))
  return items.filter((_, i) => done[i])
}
