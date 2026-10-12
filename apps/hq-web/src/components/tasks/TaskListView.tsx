import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { TaskCard } from './TaskCard'
import { groupByInitiative, nestRows } from './hierarchy'

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
  selection?: ListSelection
}

export function TaskListView({ tasks, allById, initiativeById, onSelect, selection }: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const grouped = useMemo(() => groupByInitiative(tasks, initiativeById), [tasks, initiativeById])

  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  return (
    <div className="space-y-6 pb-6">
      {grouped.map(([initiativeName, items]) => (
        <div key={initiativeName}>
          <h2 className="text-[11px] font-bold uppercase tracking-wider text-neutral-500 mb-2.5 flex items-center gap-2">
            {selection && <GroupBox name={initiativeName} items={items} selection={selection} />}
            <span>
              {initiativeName}
              <span className="text-neutral-700 ml-1.5">({items.length})</span>
            </span>
          </h2>
          <div className="space-y-2.5">
            {/* While picking, every sub-task shows, so what is ticked is always what is on screen. */}
            {nestRows(items, allById, selection ? undefined : collapsed).map(({ task, depth, parentLabel }) => (
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
                    onClick={() => toggle(task.id)}
                    className="mt-1 ml-2 text-[11px] text-neutral-500 hover:text-neutral-300 flex items-center gap-1"
                  >
                    {collapsed.has(task.id) ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
                    {task.subtask_count} sub-task{task.subtask_count === 1 ? '' : 's'}
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

/** Ticks or clears a whole initiative; half ticked shows as the dash state. */
function GroupBox({ name, items, selection }: { name: string; items: TaskItem[]; selection: ListSelection }) {
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
      aria-label={`Select all in ${name}`}
      className="w-4 h-4 shrink-0 cursor-pointer accent-emerald-400"
    />
  )
}
