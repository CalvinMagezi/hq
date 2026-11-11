import { hqJson } from './hqAuth'

export type NotificationKind = 'action_needed' | 'value_item'
export type NotificationState = 'pending' | 'approved' | 'dismissed' | 'expired'

interface NotificationMetadata {
  artifact_path?: string
  score?: number
  extra?: Record<string, string>
  /** A page of the web app the item opens, such as /tasks. */
  link?: string
  /** The dialog an agent waits at, when the item is about one. */
  dialog?: NotificationDialog
  /** The tool call the request would run, as the server recorded it. */
  approval?: ApprovalQuote
  /** Set on the item a new sign-in leaves: the device that signed in. */
  sign_in?: { device_id: string }
}

/** What a request would run, and whether allowing it loses something (a delete, an overwrite). */
export interface ApprovalQuote {
  tool: string
  args: Record<string, unknown> | null
  loses: boolean
  /** The word for the Allow button when it loses something ("delete"); null for a plain request. */
  verb: string | null
  /** The web chat that asked; null for another chat app or a chat not saved yet. */
  thread_id?: string | null
  /** The whole site a fetch request can be allowed for. */
  site?: string | null
}

/** A blocked agent's dialog as the server read it (`metadata.dialog`). */
export interface NotificationDialog {
  session_id: string
  seq: number
  kind: string
  question: string | null
  command: string | null
  options: { id: number; label: string; remember?: boolean; typed?: boolean }[]
  /** The one-time yes a button may pick; null when only the agent's own page can answer. */
  approve: { id: number; label: string | null } | null
  answered: boolean
  /** Allowing this loses something, so the button is red and names it. */
  loses?: boolean
  verb?: string | null
}

export interface NotificationItem {
  id: string
  kind: NotificationKind
  category: string
  title: string
  description: string
  state: NotificationState
  created_at: string
  metadata: NotificationMetadata
  actions: string[]
  /** The owner opened it, or marked everything read since it arrived. Absent from an older HQ. */
  read?: boolean
  /** What it is about; the owner silences a topic, never one item. */
  topic?: string
  /** Its topic is silenced: listed, but no notification and no count on the badge. */
  silenced?: boolean
}

export const SILENCE_TOPICS = ['helpers', 'tasks', 'tips', 'upkeep', 'email'] as const
export type SilenceTopic = (typeof SILENCE_TOPICS)[number]

/** The topic an item can be silenced by; null for what HQ asks itself, which always gets through. */
export function silenceTopicOf(item: Pick<NotificationItem, 'topic'>): SilenceTopic | null {
  return (SILENCE_TOPICS as readonly string[]).includes(item.topic ?? '') ? (item.topic as SilenceTopic) : null
}

export interface SilenceRow {
  topic: string
  silenced: boolean
  /** When it ends by itself; null until it is turned back on. */
  until: string | null
}

export interface Silences {
  pause: SilenceRow
  topics: SilenceRow[]
}

export function fetchSilences(): Promise<Silences> {
  return hqJson('/api/notifications/silences')
}

/** Silences a topic, or everything with `all`; without minutes it lasts until turned back on. */
export function silenceTopic(topic: SilenceTopic | 'all', minutes?: number): Promise<Silences> {
  return hqJson('/api/notifications/silences', 'POST', minutes ? { topic, minutes } : { topic })
}

export function unsilenceTopic(topic: SilenceTopic | 'all'): Promise<Silences> {
  return hqJson(`/api/notifications/silences/${encodeURIComponent(topic)}`, 'DELETE')
}

interface NotificationsResponse {
  notifications: NotificationItem[]
  unread_count: number
  pending_approvals_count: number
  /** Updates that arrived since the owner last read them. Absent from an older HQ. */
  unread_updates_count?: number
}

interface ListNotificationsFilter {
  state?: 'pending' | 'all' | 'resolved' | 'history'
  category?: 'all' | 'value' | 'system'
  limit?: number
}

export async function fetchNotificationsClient(filter?: ListNotificationsFilter): Promise<NotificationsResponse> {
  const qs = new URLSearchParams()
  if (filter?.state) qs.set('state', filter.state)
  if (filter?.category) qs.set('category', filter.category)
  if (filter?.limit) qs.set('limit', String(filter.limit))

  return hqJson(`/api/notifications?${qs.toString()}`)
}

export function executeNotificationAction(
  id: string,
  action: 'approve' | 'reject' | 'dismiss' | string
): Promise<{ success: boolean; new_state: string; message: string }> {
  return hqJson(`/api/notifications/${encodeURIComponent(id)}/action`, 'POST', { action })
}

/** Sites that stopped asking because the owner allowed them in several chats. */
export async function fetchTrustedSites(): Promise<string[]> {
  return (await hqJson<{ sites: string[] }>('/api/fetch-sites')).sites
}

export function forgetTrustedSite(site: string): Promise<unknown> {
  return hqJson(`/api/fetch-sites/${encodeURIComponent(site)}`, 'DELETE')
}

/** Marks everything waiting as read. Nothing is answered and nothing leaves its list. */
export function markAllNotificationsRead(): Promise<{ ok: boolean }> {
  return hqJson('/api/notifications/read-all', 'POST')
}

/** The owner opened one item. */
export function markNotificationRead(id: string): Promise<{ ok: boolean }> {
  return hqJson(`/api/notifications/${encodeURIComponent(id)}/read`, 'POST')
}

/** Puts every update away, into Done, and names them for Undo. A request is never touched. */
export function clearAllUpdates(): Promise<{ ok: boolean; ids?: string[] }> {
  return hqJson('/api/notifications/archive-all', 'POST')
}

/** Empties the Done list. */
export function clearDoneList(): Promise<{ ok: boolean }> {
  return hqJson('/api/notifications/clear-done', 'POST')
}

/** Puts one cleared update back (Undo of a single dismiss). HQ answers 409 when it changed since. */
export function restoreNotification(id: string): Promise<{ ok: boolean }> {
  return hqJson(`/api/notifications/${encodeURIComponent(id)}/restore`, 'POST')
}

/** Puts a set of cleared updates back (Undo of "Clear all updates"). */
export function restoreNotifications(ids: string[]): Promise<{ ok: boolean }> {
  return hqJson('/api/notifications/restore', 'POST', { ids })
}

/** The ids "Clear all updates" would clear: pending updates only, never a Needs-you request. */
/** Which Inbox tab an item belongs to: Needs you waits for an answer, Updates only tell, Done is answered or cleared. */
export type InboxTab = 'pending' | 'all' | 'resolved'

export function inInboxTab(item: Pick<NotificationItem, 'state' | 'kind'>, tab: InboxTab): boolean {
  if (tab === 'resolved') return item.state !== 'pending'
  if (item.state !== 'pending') return false
  return tab === 'pending' ? item.kind === 'action_needed' : item.kind !== 'action_needed'
}

export function clearableUpdateIds(items: NotificationItem[]): string[] {
  return items.filter((i) => i.state === 'pending' && i.kind === 'value_item').map((i) => i.id)
}
