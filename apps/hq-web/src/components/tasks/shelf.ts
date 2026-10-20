import type { Initiative, ShelveResult, TaskItem } from '~/lib/tasksApi'

/** Ages offered for "archive completed tasks older than", in days. */
export const SWEEP_AGE_OPTIONS = [14, 30, 60, 90, 180] as const
export const DEFAULT_SWEEP_DAYS = 30

/** A task is out of the everyday lists when it, its list or its parent is archived. */
export function isShelved(
  task: TaskItem,
  initiativeById: ReadonlyMap<string, Initiative>,
  taskById: ReadonlyMap<string, TaskItem>
): boolean {
  if (task.shelved_at) return true
  if (initiativeById.get(task.initiative_id)?.shelved_at) return true
  const parent = task.parent_task_id ? taskById.get(task.parent_task_id) : undefined
  return Boolean(parent?.shelved_at)
}

/** The tasks the everyday lists show: the store minus whatever is archived. */
export function withoutShelved(
  tasks: readonly TaskItem[],
  initiativeById: ReadonlyMap<string, Initiative>
): TaskItem[] {
  const taskById = new Map(tasks.map((t) => [t.id, t]))
  return tasks.filter((t) => !isShelved(t, initiativeById, taskById))
}

const noun = (n: number) => `${n} task${n === 1 ? '' : 's'}`

/** How many skipped tasks a notice names before it says "and N more". */
const SKIPS_LISTED = 3

/** One line for the result of archiving or unarchiving, naming what was left alone and why. */
export function shelveNotice(
  result: ShelveResult,
  verb: 'Archived' | 'Unarchived',
  displayIdOf: (ref: string) => string
): string {
  const head = result.changed.length > 0 ? `${verb} ${noun(result.changed.length)}.` : `Nothing ${verb.toLowerCase()}.`
  if (result.skipped.length === 0) return head
  const named = result.skipped.slice(0, SKIPS_LISTED).map((s) => `${displayIdOf(s.task)} ${s.reason}`)
  const more = result.skipped.length - named.length
  return `${head} Left alone: ${named.join('; ')}${more > 0 ? `; and ${more} more` : ''}.`
}
