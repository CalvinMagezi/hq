import { useMemo, useRef, useState } from 'react'
import { STATUS_LABELS, STATUS_ORDER, type TaskItem, type TaskStatus } from '~/lib/tasksApi'
import { t } from '~/strings'
import { TaskCard } from './TaskCard'
import { parentLabelFor } from './hierarchy'
import { usePointerDrag, type DragPoint } from './usePointerDrag'
import { boardKey, moveTargets, visibleDone } from './boardMoves'

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
  const [menuFor, setMenuFor] = useState<string | null>(null)
  const [showAllDone, setShowAllDone] = useState(false)
  const justDragged = useRef(false)
  const boardRef = useRef<HTMLDivElement>(null)

  // Left and Right go to the nearest card in the next column, Up and Down to the next card in this one.
  const moveFocus = (card: HTMLElement, dir: 'left' | 'right' | 'up' | 'down') => {
    const board = boardRef.current
    if (!board) return
    const column = card.closest<HTMLElement>('[data-board-status]')
    const cards = [...(column?.querySelectorAll<HTMLElement>('[data-board-card]') ?? [])]
    if (dir === 'up' || dir === 'down') {
      cards[cards.indexOf(card) + (dir === 'up' ? -1 : 1)]?.focus()
      return
    }
    const columns = [...board.querySelectorAll<HTMLElement>('[data-board-status]')]
    const step = dir === 'left' ? -1 : 1
    for (let i = columns.indexOf(column!) + step; i >= 0 && i < columns.length; i += step) {
      const target = columns[i].querySelector<HTMLElement>('[data-board-card]')
      if (target) return target.focus()
    }
  }

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
      justDragged.current = true
      setTimeout(() => (justDragged.current = false), 0)
      const target = statusUnderPointer(point.x, point.y)
      if (target && target !== task.status) onMove(task, target)
    },
    onCancel: () => setDrag(null),
  })

  return (
    <div ref={boardRef} className="flex gap-3 overflow-x-auto pb-6 -mx-4 px-4 sm:mx-0 sm:px-0 snap-x">
      {STATUS_ORDER.map((status) => {
        const all = columns.get(status) ?? []
        const items = status === 'complete' ? visibleDone(all, showAllDone) : all
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
              <span className="text-neutral-700 ml-1.5">({all.length})</span>
            </h2>
            <div className="space-y-2 min-h-16">
              {items.map((task) => (
                <div
                  key={task.id}
                  onPointerDown={(e) => {
                    if ((e.target as HTMLElement).closest('[data-board-move]')) return
                    startDrag(e, task)
                  }}
                  onClickCapture={(e) => {
                    if (justDragged.current) {
                      e.preventDefault()
                      e.stopPropagation()
                    }
                  }}
                  onKeyDown={(e) => {
                    const action = boardKey(e.key)
                    if (!action || e.ctrlKey || e.metaKey || e.altKey) return
                    const card = (e.target as HTMLElement).closest<HTMLElement>('[data-board-card]')
                    if (!card || (e.target as HTMLElement).closest('[data-board-move], [role="menu"]')) return
                    e.preventDefault()
                    if (action === 'move') setMenuFor(task.id)
                    else moveFocus(card, action)
                  }}
                  className={`relative select-none [-webkit-touch-callout:none] ${drag?.task.id === task.id ? 'opacity-30' : ''}`}
                >
                  <span data-board-card className="contents">
                    <TaskCard task={task} parentLabel={parentLabelFor(task, allById)} compact asLink onSelect={onSelect} />
                  </span>
                  <button
                    type="button"
                    data-board-move
                    aria-haspopup="menu"
                    aria-expanded={menuFor === task.id}
                    aria-label={t('board_move_card', { title: task.title })}
                    onClick={() => setMenuFor(menuFor === task.id ? null : task.id)}
                    className="mt-1 w-full min-h-11 rounded-lg text-xs text-neutral-300 border border-white/10 px-2"
                  >
                    {t('board_move')}
                  </button>
                  {menuFor === task.id && (
                    <div role="menu" className="mt-1 flex flex-col rounded-lg border border-white/10 p-1" style={{ background: 'var(--bg-solid-elevated)' }}>
                      {moveTargets(task).map((s) => (
                        <button
                          key={s}
                          type="button"
                          role="menuitem"
                          onClick={() => {
                            setMenuFor(null)
                            onMove(task, s)
                          }}
                          className="text-left min-h-11 px-3 rounded-md text-xs text-neutral-200 hover:bg-white/10"
                        >
                          {STATUS_LABELS[s]}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              ))}
              {status === 'complete' && items.length < all.length && (
                <button type="button" onClick={() => setShowAllDone(true)} className="w-full min-h-11 text-xs text-emerald-400">
                  {t('board_show_all', { n: all.length })}
                </button>
              )}
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
