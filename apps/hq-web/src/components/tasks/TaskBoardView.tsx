import { useMemo, useState } from 'react'
import { STATUS_LABELS, STATUS_ORDER, type TaskItem, type TaskStatus } from '~/lib/tasksApi'
import { TaskCard } from './TaskCard'
import { parentLabelFor } from './hierarchy'
import { usePointerDrag, type DragPoint } from './usePointerDrag'

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  onSelect: (task: TaskItem) => void
  onMove: (task: TaskItem, status: TaskStatus) => void
}

// Keeps the floating card slightly offset so the pointer stays over the column below it.
const GHOST_OFFSET_X = 24
const GHOST_OFFSET_Y = 16

interface DragState {
  task: TaskItem
  point: DragPoint
  overStatus: TaskStatus | null
}

function statusUnderPointer(x: number, y: number): TaskStatus | null {
  const column = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-board-status]')
  return (column?.dataset.boardStatus as TaskStatus | undefined) ?? null
}

export function TaskBoardView({ tasks, allById, onSelect, onMove }: Props) {
  const [drag, setDrag] = useState<DragState | null>(null)

  const columns = useMemo(() => {
    const byStatus = new Map<TaskStatus, TaskItem[]>(STATUS_ORDER.map((s) => [s, []]))
    for (const t of tasks) byStatus.get(t.status)?.push(t)
    return byStatus
  }, [tasks])

  const startDrag = usePointerDrag<TaskItem>({
    onStart: (task, point) => setDrag({ task, point, overStatus: task.status }),
    onMove: (task, point) => setDrag({ task, point, overStatus: statusUnderPointer(point.x, point.y) }),
    onEnd: (task, point) => {
      setDrag(null)
      const target = statusUnderPointer(point.x, point.y)
      if (target && target !== task.status) onMove(task, target)
    },
    onCancel: () => setDrag(null),
    onClick: onSelect,
  })

  return (
    <div className="flex gap-3 overflow-x-auto pb-6 -mx-4 px-4 sm:mx-0 sm:px-0 snap-x">
      {STATUS_ORDER.map((status) => {
        const items = columns.get(status) ?? []
        const isTarget = drag !== null && drag.overStatus === status && drag.task.status !== status
        return (
          <div
            key={status}
            data-board-status={status}
            className={`flex flex-col w-[260px] shrink-0 snap-start rounded-2xl border p-2.5 transition-colors ${
              isTarget ? 'border-emerald-500/40 bg-emerald-500/5' : 'hq-glass-card'
            }`}
          >
            <h2 className="text-[11px] font-bold uppercase tracking-wider text-neutral-500 px-1.5 mb-2.5">
              {STATUS_LABELS[status]}
              <span className="text-neutral-700 ml-1.5">({items.length})</span>
            </h2>
            <div className="space-y-2 min-h-16">
              {items.map((task) => (
                <div
                  key={task.id}
                  onPointerDown={(e) => startDrag(e, task)}
                  className={`select-none [-webkit-touch-callout:none] ${drag?.task.id === task.id ? 'opacity-30' : ''}`}
                >
                  <TaskCard task={task} parentLabel={parentLabelFor(task, allById)} compact />
                </div>
              ))}
            </div>
          </div>
        )
      })}

      {drag && (
        <div
          className="fixed z-50 pointer-events-none w-[240px] rotate-2 shadow-2xl"
          style={{ left: drag.point.x - GHOST_OFFSET_X, top: drag.point.y - GHOST_OFFSET_Y }}
        >
          <TaskCard task={drag.task} compact />
        </div>
      )}
    </div>
  )
}
