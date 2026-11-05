import type { TaskItem } from './tasksApi'

/** Open tasks that wait on this one: marking it done tells them they can start, so it asks first. */
export function dependentsOf(task: Pick<TaskItem, 'id'>, all: Iterable<TaskItem>): TaskItem[] {
  return [...all].filter((x) => !x.archived_at && x.status !== 'complete' && x.depends_on.includes(task.id))
}

/** Whether a status change is a plain one-task done that needs the dependents question. */
export function asksBeforeDone(previous: TaskItem | undefined, patch: Record<string, unknown>, all: Iterable<TaskItem>): number {
  if (patch.status !== 'complete' || !previous || previous.status === 'complete') return 0
  return dependentsOf(previous, all).length
}

export type DoneQuestion = 'check' | 'dependents' | null

/** Which question a plain one-task done asks first: a pending check (P4), then waiting tasks (P3). */
export function doneQuestion(
  previous: TaskItem | undefined,
  patch: Record<string, unknown>,
  all: Iterable<TaskItem>,
  checkPending: boolean,
): DoneQuestion {
  if (patch.status !== 'complete' || !previous || previous.status === 'complete') return null
  if (checkPending) return 'check'
  return dependentsOf(previous, all).length > 0 ? 'dependents' : null
}
