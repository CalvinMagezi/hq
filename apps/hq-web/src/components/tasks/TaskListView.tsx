import { useMemo, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { TaskCard } from './TaskCard'
import { groupByInitiative, nestRows } from './hierarchy'

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  initiativeById: Map<string, Initiative>
  onSelect: (task: TaskItem) => void
}

export function TaskListView({ tasks, allById, initiativeById, onSelect }: Props) {
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
          <h2 className="text-[11px] font-bold uppercase tracking-wider text-neutral-500 mb-2.5">
            {initiativeName}
            <span className="text-neutral-700 ml-1.5">({items.length})</span>
          </h2>
          <div className="space-y-2.5">
            {nestRows(items, allById, collapsed).map(({ task, depth, parentLabel }) => (
              <div key={task.id} className={depth === 1 ? 'ml-3 sm:ml-6 pl-2 sm:pl-3 border-l border-white/10' : undefined}>
                <TaskCard task={task} onSelect={onSelect} parentLabel={parentLabel} />
                {depth === 0 && task.subtask_count > 0 && (
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
