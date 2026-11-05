import type { TaskItem, TaskStatus } from './tasksApi'

export type TaskTab = 'today' | 'upcoming' | 'all'
export type TaskChip = 'active' | 'stuck' | 'ready' | 'done' | 'archived'

export const TABS: readonly TaskTab[] = ['today', 'upcoming', 'all']
export const CHIPS: readonly TaskChip[] = ['active', 'stuck', 'ready', 'done', 'archived']

const CHIP_STATUS: Partial<Record<TaskChip, TaskStatus>> = { stuck: 'blocked', ready: 'ready_for_review', done: 'complete' }

/** The one status a chip narrows to; null for Active (everything open) and Archived. */
export const chipStatus = (chip: TaskChip): TaskStatus | null => CHIP_STATUS[chip] ?? null

/** Today: open and due by today, or already started. Upcoming: open and dated after today. All: open or not, the chips decide. */
export function matchesTab(task: TaskItem, tab: TaskTab, today: string): boolean {
  if (tab === 'all') return true
  if (task.status === 'complete') return false
  const due = task.due_date
  const start = task.start_date
  if (tab === 'today') return task.status === 'in_progress' || (!!due && due <= today) || (!!start && start <= today)
  return (!!due && due > today) || (!!start && start > today)
}
