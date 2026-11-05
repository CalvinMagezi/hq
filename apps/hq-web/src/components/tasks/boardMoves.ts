import { STATUS_ORDER, type TaskItem, type TaskStatus } from '~/lib/tasksApi'

/** How many finished tasks the Done column shows before "Show all". */
export const DONE_SHOWN = 5

/** The columns a card can move to. Ready to check is not offered for a task nobody has finished. */
export function moveTargets(task: Pick<TaskItem, 'status' | 'first_ready_for_review_at'>): TaskStatus[] {
  return STATUS_ORDER.filter((s) => {
    if (s === task.status) return false
    if (s === 'ready_for_review') return task.first_ready_for_review_at !== null
    return true
  })
}

export type BoardKey = 'move' | 'left' | 'right' | 'up' | 'down'

/** What a key does on a focused card: M opens its Move menu, the arrows go between cards and columns. */
export function boardKey(key: string): BoardKey | null {
  switch (key) {
    case 'm':
    case 'M':
      return 'move'
    case 'ArrowLeft':
      return 'left'
    case 'ArrowRight':
      return 'right'
    case 'ArrowUp':
      return 'up'
    case 'ArrowDown':
      return 'down'
    default:
      return null
  }
}

/** The Done column's visible cards: the newest few, or all of them once expanded. */
export function visibleDone(items: TaskItem[], expanded: boolean): TaskItem[] {
  if (expanded || items.length <= DONE_SHOWN) return items
  return [...items]
    .sort((a, b) => (b.completed_at ?? b.updated_at).localeCompare(a.completed_at ?? a.updated_at))
    .slice(0, DONE_SHOWN)
}
