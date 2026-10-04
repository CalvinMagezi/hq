import type { TaskItem } from './tasksApi'
import { compareTasksByActivity } from '~/components/tasks/hierarchy'

export const HOME_TASK_LIMIT = 8

export type SectionState<T> =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'empty' }
  | { kind: 'ready'; items: T[] }

interface QueryLike<T> {
  data: T[] | undefined
  isPending: boolean
  isError: boolean
  error: unknown
}

/** Saved data wins over an error, so a failed refresh never hides what is already on screen. */
export function sectionState<T>(q: QueryLike<T>): SectionState<T> {
  if (q.data && q.data.length > 0) return { kind: 'ready', items: q.data }
  if (q.isError) return { kind: 'error', message: q.error instanceof Error ? q.error.message : 'Request failed' }
  if (q.isPending) return { kind: 'loading' }
  return { kind: 'empty' }
}

export function inProgressTasks(tasks: TaskItem[]): TaskItem[] {
  return tasks
    .filter((t) => t.status === 'in_progress')
    .sort(compareTasksByActivity)
    .slice(0, HOME_TASK_LIMIT)
}
