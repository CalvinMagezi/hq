import { useCallback, useEffect, useMemo, useState } from 'react'
import type { TaskItem } from '~/lib/tasksApi'

/** Toggles every id in `ids` on or off in a copy of `current`. */
export function withIds(current: ReadonlySet<string>, ids: readonly string[], on: boolean): Set<string> {
  const next = new Set(current)
  for (const id of ids) {
    if (on) next.add(id)
    else next.delete(id)
  }
  return next
}

/** `current` limited to the ids in `visible`; the same set back when nothing was dropped. */
export function keepVisible(current: ReadonlySet<string>, visible: readonly TaskItem[]): ReadonlySet<string> {
  const shown = new Set(visible.map((t) => t.id))
  const kept = new Set([...current].filter((id) => shown.has(id)))
  return kept.size === current.size ? current : kept
}

/**
 * Which of the tasks on screen are ticked for a bulk action.
 *
 * Only tasks that are still on screen count: filter a ticked task away, or watch
 * it leave the list after a status change, and it drops out, so an action never
 * reaches a row the person cannot see.
 */
export function useTaskSelection(visible: readonly TaskItem[], enabled: boolean) {
  const [on, setOn] = useState(false)
  const [ids, setIds] = useState<ReadonlySet<string>>(new Set())
  const active = on && enabled

  useEffect(() => {
    setIds((current) => (current.size === 0 ? current : keepVisible(current, visible)))
  }, [visible])

  // Leaving the list view (or the archive) ends selecting, so it never comes back half remembered.
  useEffect(() => {
    if (enabled) return
    setOn(false)
    setIds((current) => (current.size === 0 ? current : new Set()))
  }, [enabled])

  const selected = useMemo(() => visible.filter((t) => ids.has(t.id)), [visible, ids])

  const start = useCallback(() => setOn(true), [])
  const stop = useCallback(() => {
    setOn(false)
    setIds(new Set())
  }, [])
  const toggle = useCallback((task: TaskItem) => setIds((c) => withIds(c, [task.id], !c.has(task.id))), [])
  const setMany = useCallback(
    (tasks: readonly TaskItem[], ticked: boolean) => setIds((c) => withIds(c, tasks.map((t) => t.id), ticked)),
    []
  )
  const clear = useCallback(() => setIds(new Set()), [])

  return { active, selected, selectedIds: ids, start, stop, toggle, setMany, clear }
}
