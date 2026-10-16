import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { TaskCard } from './TaskCard'
import { groupByInitiative, nestRows } from './hierarchy'

/** Which initiative sections are folded away, and how to change that. */
export interface ListSections {
  collapsed: ReadonlySet<string>
  onToggle: (name: string) => void
  /** Folds or unfolds every section named, leaving the rest alone. */
  onSetMany: (names: string[], collapsed: boolean) => void
}

/** Picking tasks for a bulk action: cards tick instead of opening, and every group gets a box. */
export interface ListSelection {
  ticked: ReadonlySet<string>
  onToggle: (task: TaskItem) => void
  onToggleMany: (tasks: TaskItem[], ticked: boolean) => void
}

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  initiativeById: Map<string, Initiative>
  onSelect: (task: TaskItem) => void
  sections: ListSections
  selection?: ListSelection
}

export function TaskListView({ tasks, allById, initiativeById, onSelect, sections, selection }: Props) {
  // Parents whose sub-tasks are folded away (not to be confused with the folded sections below).
  const [foldedParents, setFoldedParents] = useState<Set<string>>(new Set())
  const grouped = useMemo(() => groupByInitiative(tasks, initiativeById), [tasks, initiativeById])

  const toggleParent = (id: string) =>
    setFoldedParents((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const names = grouped.map(([name]) => name)
  const foldedCount = names.filter((n) => sections.collapsed.has(n)).length

  return (
    <div className="space-y-6 pb-6">
      {names.length > 1 && (
        <div className="flex justify-end gap-4 mb-3 text-[11px] font-semibold">
          <button
            type="button"
            disabled={foldedCount === names.length}
            onClick={() => sections.onSetMany(names, true)}
            className="text-neutral-500 hover:text-neutral-200 disabled:opacity-40 disabled:pointer-events-none"
          >
            Collapse all
          </button>
          <button
            type="button"
            disabled={foldedCount === 0}
            onClick={() => sections.onSetMany(names, false)}
            className="text-neutral-500 hover:text-neutral-200 disabled:opacity-40 disabled:pointer-events-none"
          >
            Expand all
          </button>
        </div>
      )}
      {grouped.map(([initiativeName, items]) => {
        const folded = sections.collapsed.has(initiativeName)
        const bodyId = `section-${initiativeName}`
        return (
          <div key={initiativeName}>
            <h2 className="text-[11px] font-bold uppercase tracking-wider text-neutral-500 mb-2.5 flex items-center gap-2">
              {selection && <GroupBox name={initiativeName} items={items} selection={selection} disabled={folded} />}
              <button
                type="button"
                aria-expanded={!folded}
                aria-controls={bodyId}
                onClick={() => sections.onToggle(initiativeName)}
                className="flex items-center gap-1.5 min-w-0 flex-1 py-1 text-left uppercase tracking-wider hover:text-neutral-300"
              >
                {folded ? <ChevronRight className="w-3.5 h-3.5 shrink-0" /> : <ChevronDown className="w-3.5 h-3.5 shrink-0" />}
                <span className="truncate">{initiativeName}</span>
                <span className="text-neutral-700 shrink-0">({items.length})</span>
              </button>
            </h2>
            {!folded && (
              <div id={bodyId} className="space-y-2.5">
                {/* While picking, every sub-task shows, so what is ticked is always what is on screen. */}
                {nestRows(items, allById, selection ? undefined : foldedParents).map(({ task, depth, parentLabel }) => (
                  <div key={task.id} className={depth === 1 ? 'ml-3 sm:ml-6 pl-2 sm:pl-3 border-l border-white/10' : undefined}>
                    <TaskCard
                      task={task}
                      onSelect={onSelect}
                      parentLabel={parentLabel}
                      select={selection && { ticked: selection.ticked.has(task.id), onToggle: selection.onToggle }}
                    />
                    {!selection && depth === 0 && task.subtask_count > 0 && (
                      <button
                        type="button"
                        onClick={() => toggleParent(task.id)}
                        className="mt-1 ml-2 text-[11px] text-neutral-500 hover:text-neutral-300 flex items-center gap-1"
                      >
                        {foldedParents.has(task.id) ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
                        {task.subtask_count} sub-task{task.subtask_count === 1 ? '' : 's'}
                      </button>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

interface GroupBoxProps {
  name: string
  items: TaskItem[]
  selection: ListSelection
  /** A folded section's tasks are not on screen, so they cannot be ticked from here. */
  disabled: boolean
}

/** Ticks or clears a whole initiative; half ticked shows as the dash state. */
function GroupBox({ name, items, selection, disabled }: GroupBoxProps) {
  const ref = useRef<HTMLInputElement>(null)
  const ticked = items.filter((t) => selection.ticked.has(t.id)).length
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = ticked > 0 && ticked < items.length
  }, [ticked, items.length])
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={ticked === items.length}
      onChange={(e) => selection.onToggleMany(items, e.target.checked)}
      disabled={disabled}
      aria-label={`Select all in ${name}`}
      title={disabled ? 'Expand the section to select its tasks' : undefined}
      className="w-4 h-4 shrink-0 cursor-pointer accent-emerald-400 disabled:cursor-not-allowed disabled:opacity-40"
    />
  )
}
