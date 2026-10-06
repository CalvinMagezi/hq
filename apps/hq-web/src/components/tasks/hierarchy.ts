import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { parseSqliteUtc } from '~/lib/tasksApi'

export interface TaskRow {
  task: TaskItem
  depth: 0 | 1
  /** Set when a sub-task is shown without its parent (the parent was filtered out). */
  parentLabel?: string
}

export function parentLabelFor(task: TaskItem, byId: Map<string, TaskItem>): string | undefined {
  if (!task.parent_task_id) return undefined
  const parent = byId.get(task.parent_task_id)
  return parent ? `${parent.display_id} ${parent.title}` : undefined
}

function taskActivityTime(task: TaskItem): number {
  const raw = task.updated_at || task.created_at
  if (!raw) return 0
  const t = parseSqliteUtc(raw).getTime()
  return Number.isNaN(t) ? 0 : t
}

function taskCreatedTime(task: TaskItem): number {
  const raw = task.created_at
  if (!raw) return 0
  const t = parseSqliteUtc(raw).getTime()
  return Number.isNaN(t) ? 0 : t
}

/**
 * Compares tasks by latest activity descending (updated_at, then created_at),
 * with deterministic tie-breaking on display_id then id.
 */
export function compareTasksByActivity(a: TaskItem, b: TaskItem): number {
  const aAct = taskActivityTime(a)
  const bAct = taskActivityTime(b)
  if (bAct !== aAct) {
    return bAct - aAct
  }
  const aCreated = taskCreatedTime(a)
  const bCreated = taskCreatedTime(b)
  if (bCreated !== aCreated) {
    return bCreated - aCreated
  }
  const idCompare = (b.display_id || '').localeCompare(a.display_id || '')
  if (idCompare !== 0) return idCompare
  return (b.id || '').localeCompare(a.id || '')
}

/**
 * Groups tasks by initiative name. Initiative groups are sorted by the latest
 * activity of their tasks (newest task group first), with deterministic tie-break
 * on initiative name. Tasks within each group are ordered by activity descending.
 */
export function groupByInitiative(
  tasks: TaskItem[],
  initiativeById: Map<string, Initiative>
): [string, TaskItem[]][] {
  const groups = new Map<string, TaskItem[]>()
  for (const t of tasks) {
    const name = initiativeById.get(t.initiative_id)?.name ?? 'Other'
    const list = groups.get(name) ?? []
    list.push(t)
    groups.set(name, list)
  }

  // Sort tasks within each initiative group by activity descending
  for (const list of groups.values()) {
    list.sort(compareTasksByActivity)
  }

  return Array.from(groups.entries()).sort((a, b) => {
    const aNewest = a[1].length > 0 ? taskActivityTime(a[1][0]) : 0
    const bNewest = b[1].length > 0 ? taskActivityTime(b[1][0]) : 0
    if (bNewest !== aNewest) {
      return bNewest - aNewest
    }
    return a[0].localeCompare(b[0])
  })
}

/**
 * Orders `tasks` so each sub-task follows its parent. A sub-task whose parent
 * is not in `tasks` stays at the top level and carries its parent's label.
 * Sibling subtasks are ordered by activity descending.
 */
export function nestRows(tasks: TaskItem[], allById: Map<string, TaskItem>, collapsed?: Set<string>): TaskRow[] {
  const visible = new Set(tasks.map((t) => t.id))
  const children = new Map<string, TaskItem[]>()
  for (const t of tasks) {
    if (!t.parent_task_id || !visible.has(t.parent_task_id)) continue
    const list = children.get(t.parent_task_id) ?? []
    list.push(t)
    children.set(t.parent_task_id, list)
  }

  const rows: TaskRow[] = []
  for (const task of tasks) {
    if (task.parent_task_id && visible.has(task.parent_task_id)) continue
    rows.push({ task, depth: 0, parentLabel: parentLabelFor(task, allById) })
    if (collapsed?.has(task.id)) continue
    const kids = (children.get(task.id) ?? []).sort(compareTasksByActivity)
    for (const child of kids) rows.push({ task: child, depth: 1 })
  }
  return rows
}
