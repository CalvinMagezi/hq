import { HqHttpError } from '~/lib/hqAuth'
import { HTTP_CONFLICT, type TaskItem } from '~/lib/tasksApi'

/**
 * Requests in flight at once. Every task is its own PATCH and its own write
 * transaction on the server, so a few at a time keeps a long selection quick
 * without piling a hundred requests onto the write lock.
 */
export const BULK_CONCURRENCY = 4

/** How many failed tasks a notice names before it says "and N more". */
const FAILURES_LISTED = 3

export interface BulkFailure {
  task: TaskItem
  error: unknown
}

export interface BulkOutcome<R> {
  done: { task: TaskItem; value: R }[]
  failed: BulkFailure[]
}

/**
 * Runs `op` on every task, `concurrency` at a time. One task failing never stops
 * the rest: it lands in `failed` and the others carry on.
 */
export async function runBulk<R>(
  tasks: readonly TaskItem[],
  op: (task: TaskItem) => Promise<R>,
  onProgress?: (settled: number, total: number) => void,
  concurrency = BULK_CONCURRENCY
): Promise<BulkOutcome<R>> {
  const outcome: BulkOutcome<R> = { done: [], failed: [] }
  let next = 0
  let settled = 0
  const worker = async () => {
    while (next < tasks.length) {
      const task = tasks[next++]
      try {
        outcome.done.push({ task, value: await op(task) })
      } catch (error) {
        outcome.failed.push({ task, error })
      }
      onProgress?.(++settled, tasks.length)
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, tasks.length) }, worker))
  return outcome
}

export function failureReason(error: unknown): string {
  if (error instanceof HqHttpError && error.status === HTTP_CONFLICT) return 'was changed by someone else'
  return error instanceof Error ? error.message : 'could not be updated'
}

/** One line per bulk run, for the notice banner. Null when nothing failed. */
export function failureNotice(failed: readonly BulkFailure[]): string | null {
  if (failed.length === 0) return null
  const named = failed.slice(0, FAILURES_LISTED).map((f) => `${f.task.display_id} ${failureReason(f.error)}`)
  const more = failed.length - named.length
  const list = named.join('; ') + (more > 0 ? `; and ${more} more` : '')
  return `${failed.length} could not be updated: ${list}.`
}

const noun = (n: number) => `${n} task${n === 1 ? '' : 's'}`

/**
 * What a bulk run did, in one line. `attempted` is the tasks that needed a
 * change, `unchanged` the ones already at the target and left alone.
 * `phrase` finishes the sentence, e.g. "moved to Complete".
 */
export function bulkMessage(changed: number, attempted: number, unchanged: number, phrase: string): string {
  if (attempted === 0) return `Nothing to change, ${noun(unchanged)} already there`
  const head = changed === attempted ? noun(changed) : `${changed} of ${noun(attempted)}`
  const tail = unchanged > 0 ? `, ${unchanged} already there` : ''
  return `${head} ${phrase}${tail}`
}

/** The tasks a change would actually touch, and how many were already at the target. */
export function splitUnchanged(
  tasks: readonly TaskItem[],
  isAtTarget: (task: TaskItem) => boolean
): { toChange: TaskItem[]; unchanged: number } {
  const toChange = tasks.filter((t) => !isAtTarget(t))
  return { toChange, unchanged: tasks.length - toChange.length }
}

/** Soft-dependency warnings from many updates, each said once. */
export function uniqueWarnings(warnings: readonly (readonly string[] | undefined)[]): string[] {
  return [...new Set(warnings.flatMap((w) => w ?? []))]
}
