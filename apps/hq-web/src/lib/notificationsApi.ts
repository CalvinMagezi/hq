import { hqJson } from './hqAuth'

export type NotificationKind = 'action_needed' | 'value_item'
export type NotificationState = 'pending' | 'approved' | 'dismissed' | 'expired'

interface NotificationMetadata {
  artifact_path?: string
  score?: number
  extra?: Record<string, string>
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
