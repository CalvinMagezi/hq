// What an Inbox card is and says (docs/plans/ux-run/web/wave-4/05-inbox.md): its kind, whether the
// request ran out, why an answer failed, and the notifications card's 7 day "Not now".

import { t } from '~/strings'
import { dateByAge, displayZone } from './time'
import type { NotificationItem } from './notificationsApi'

export type CardKind = 'sign_in' | 'question' | 'approval' | 'task_check' | 'stuck' | 'update'

/** A request waits 24 hours for its answer, then lapses (resolutions.md C-7). */
export const REQUEST_LIFETIME_MS = 24 * 60 * 60 * 1000
export const NOTIFY_HIDE_MS = 7 * 24 * 60 * 60 * 1000
export const NOTIFY_HIDDEN_KEY = 'hq-notify-card-until'

const HTTP_CONFLICT = 409
/** The source of HQ's items about a helper's session (`session_supervisor/alerts.rs`). */
const HELPER_SUPERVISOR_SOURCE = 'session-supervisor'

export function cardKind(n: NotificationItem): CardKind {
  if (n.metadata.sign_in) return 'sign_in'
  if (n.metadata.dialog) return 'question'
  if (n.metadata.approval) return 'approval'
  const source = n.metadata.extra?.source_task
  if (source === 'task_ready_for_review') return 'task_check'
  // A helper waiting at a screen HQ cannot read (or keeps dismissing): the session supervisor's item
  // without a dialog it could read.
  if (source === HELPER_SUPERVISOR_SOURCE) return 'stuck'
  // Only an item HQ can approve is an approval; a notice to look at is an update.
  return n.kind === 'action_needed' && n.actions.includes('approve') ? 'approval' : 'update'
}

/** An approval or a helper's question nobody answered in 24 hours: no buttons, just the line. */
export function lapsed(n: NotificationItem, now: number = Date.now()): boolean {
  if (n.state !== 'pending') return false
  const kind = cardKind(n)
  if (kind !== 'approval' && kind !== 'question') return false
  const made = new Date(n.created_at).getTime()
  return Number.isFinite(made) && now - made > REQUEST_LIFETIME_MS
}

/** The words for a failed answer: refused (the question changed) is not failed (HQ did not answer). */
export function answerErrorLine(err: unknown): string {
  const status = (err as { status?: number } | null)?.status
  if (status === HTTP_CONFLICT) return t('fail_send_answer_changed')
  if (err instanceof TypeError) return t('fail_send_answer_unreachable')
  if (typeof status === 'number') return t('fail_send_answer_no_reply')
  return t('fail_answer_refused')
}

export function notifyCardHidden(now: number = Date.now(), store: Pick<Storage, 'getItem'> = localStorage): boolean {
  try {
    return now < Number(store.getItem(NOTIFY_HIDDEN_KEY) ?? 0)
  } catch {
    return false
  }
}

/** Not now hides the card for 7 days. */
export function hideNotifyCard(now: number = Date.now(), store: Pick<Storage, 'setItem'> = localStorage) {
  try {
    store.setItem(NOTIFY_HIDDEN_KEY, String(now + NOTIFY_HIDE_MS))
  } catch {
    // Blocked storage: the card returns next visit.
  }
}

/** The line that leads an item a server HQ sent; null for this HQ's own. */
export function serverHqLine(n: NotificationItem): string | null {
  const x = n.metadata.extra
  if (!x?.server_hq) return null
  return x.server_hq_name ? t('inbox_on_server_hq', { name: x.server_hq_name }) : t('inbox_on_server_hq_bare')
}

const DAY_MS = 24 * 60 * 60 * 1000

function dayStamp(ms: number, zone: string): number {
  const p = new Intl.DateTimeFormat('en-CA', { timeZone: zone }).format(ms)
  return Date.parse(`${p}T00:00:00Z`)
}

/** Heading for one day of the Done tab: Today, Yesterday, then the date. */
export function dayHeading(createdAt: string, now: number = Date.now(), zone: string = displayZone()): string {
  const ms = new Date(createdAt).getTime()
  if (!Number.isFinite(ms)) return t('time_unknown')
  const days = Math.round((dayStamp(now, zone) - dayStamp(ms, zone)) / DAY_MS)
  if (days <= 0) return t('chat_group_today')
  return dateByAge(ms, now, zone)
}

/** Items in the order given, split under one heading per day. */
export function groupByDay(items: NotificationItem[], now: number = Date.now(), zone: string = displayZone()): { heading: string; items: NotificationItem[] }[] {
  const groups: { heading: string; items: NotificationItem[] }[] = []
  for (const n of items) {
    const heading = dayHeading(n.created_at, now, zone)
    const last = groups[groups.length - 1]
    if (last?.heading === heading) last.items.push(n)
    else groups.push({ heading, items: [n] })
  }
  return groups
}
