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
}

interface NotificationsResponse {
  notifications: NotificationItem[]
  unread_count: number
  pending_approvals_count: number
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

export function markAllNotificationsRead(): Promise<{ ok: boolean }> {
  return hqJson('/api/notifications/mark-all-read', 'POST')
}
