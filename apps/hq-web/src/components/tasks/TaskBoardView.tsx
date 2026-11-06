import { MoreHorizontal } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { STATUS_LABELS, STATUS_ORDER, type TaskItem, type TaskStatus } from '~/lib/tasksApi'
import { t } from '~/strings'
import { lateLabel } from './TaskCard'
import './tasks.css'
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

  const goTo = (status: TaskStatus) => {
    const column: HTMLElement | null | undefined = boardRef.current?.querySelector(`[data-board-status="${status}"]`)
    column?.scrollIntoView({ behavior: 'smooth', inline: 'start', block: 'nearest' })
  }

  const cardBody = (task: TaskItem) => {
    const late = lateLabel(task)
    const parent = parentLabelFor(task, allById)
    return (
      <>
        <span className="a-trow-title">{task.title}</span>
        <span className="a-trow-meta">
          {[task.display_id, parent, task.due_date].filter(Boolean).join(' · ')}
          {late?.late && (
            <>
              {' · '}
              <b>{late.text}</b>
            </>
          )}
        </span>
      </>
    )
  }

  return (
    <>
      <div className="bd-jump">
        {STATUS_ORDER.map((status) => (
          <button key={status} type="button" className="a-chip" onClick={() => goTo(status)}>
            {STATUS_LABELS[status]} <em>{(columns.get(status) ?? []).length}</em>
          </button>
        ))}
      </div>
      <div ref={boardRef} className="bd">
        {STATUS_ORDER.map((status) => {
          const all = columns.get(status) ?? []
          const items = status === 'complete' ? visibleDone(all, showAllDone) : all
          const isTarget = drag !== null && drag.overStatus === status && drag.task.status !== status
          const needs = status === 'blocked' || status === 'ready_for_review'
          return (
            <div key={status} data-board-status={status} className={`bd-col${needs ? ' needs' : ''}${isTarget ? ' is-over' : ''}`}>
              <div className="bd-head">
                <h3>
                  {STATUS_LABELS[status]} ({all.length})
                </h3>
              </div>
              {items.map((task) => (
                <div
                  key={task.id}
                  onPointerDown={(e) => {
                    if ((e.target as HTMLElement).closest('[data-board-move], [role="menu"]')) return
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
                  className={`bd-card${drag?.task.id === task.id ? ' is-dragging' : ''}`}
                >
                  <a
                    data-board-card
                    href={`/tasks/${encodeURIComponent(task.id)}`}
                    className="a-card"
                    onClick={(e) => {
                      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
                      e.preventDefault()
                      onSelect(task)
                    }}
                  >
                    {cardBody(task)}
                  </a>
                  <button
                    type="button"
                    data-board-move
                    aria-haspopup="menu"
                    aria-expanded={menuFor === task.id}
                    aria-label={t('board_move_card', { title: task.title })}
                    onClick={() => setMenuFor(menuFor === task.id ? null : task.id)}
                    className="a-ib"
                  >
                    <MoreHorizontal className="a-ic" aria-hidden="true" />
                  </button>
                  {menuFor === task.id && (
                    <div role="menu" className="a-menu">
                      <div className="a-grp">{t('board_move')}</div>
                      {moveTargets(task).map((s) => (
                        <button
                          key={s}
                          type="button"
                          role="menuitem"
                          className="a-menu-item"
                          onClick={() => {
                            setMenuFor(null)
                            onMove(task, s)
                          }}
                        >
                          {STATUS_LABELS[s]}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              ))}
              {status === 'complete' && items.length < all.length && (
                <button type="button" onClick={() => setShowAllDone(true)} className="a-btn compact quiet">
                  {t('board_show_all', { n: all.length })}
                </button>
              )}
            </div>
          )
        })}

        {drag && (
          <div className="bd-ghost bd-card" style={{ left: drag.point.x - GHOST_OFFSET_X, top: drag.point.y - GHOST_OFFSET_Y }}>
            <div className="a-card">{cardBody(drag.task)}</div>
          </div>
        )}
      </div>
    </>
  )
}
