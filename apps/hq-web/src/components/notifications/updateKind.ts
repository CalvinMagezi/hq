import type { NotificationItem } from '~/lib/notificationsApi'

export type UpdateKind = 'tasks' | 'helpers' | 'hq'
export type UpdateFilter = 'all' | UpdateKind

export const UPDATE_FILTERS: UpdateFilter[] = ['all', 'tasks', 'helpers', 'hq']

const TASK_REVIEW_SOURCE = 'task_ready_for_review'
const HELPER_SUPERVISOR_SOURCE = 'session-supervisor'

/** Which chip an item counts under, from what it already carries; anything unclear is HQ's own. */
export function updateKind(n: NotificationItem): UpdateKind {
  const source = n.metadata.extra?.source_task
  const link = n.metadata.link ?? ''
  if (source === TASK_REVIEW_SOURCE || link.startsWith('/tasks')) return 'tasks'
  if (source === HELPER_SUPERVISOR_SOURCE || link.startsWith('/helpers') || n.metadata.dialog) return 'helpers'
  return 'hq'
}

export function countByKind(items: NotificationItem[]): Record<UpdateFilter, number> {
  const counts: Record<UpdateFilter, number> = { all: items.length, tasks: 0, helpers: 0, hq: 0 }
  for (const n of items) counts[updateKind(n)] += 1
  return counts
}
