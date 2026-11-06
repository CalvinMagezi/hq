import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarDays, TriangleAlert } from 'lucide-react'
import { STATUS_LABELS, type Initiative, type TaskItem, type UpdateTaskInput, type WorkSession } from '~/lib/tasksApi'
import './tasks.css'
import { groupByInitiative, nestRows, type TaskRow } from './hierarchy'
import { t } from '~/strings'
import { usePersistedState } from '~/lib/usePersistedState'
import { ZOOM_KEY, isZoom } from './gantt'
import { dayOfMonth, dayOfWeek, isWeekend, monthLabel, todayDay } from './dates'
import {
  BAR_INSET,
  DAY_PX,
  MILESTONE_SIZE,
  ROW_HEIGHT,
  applyDrag,
  arrowPath,
  placementPatch,
  spanOf,
  spanToPatch,
  monthRange,
  weekRange,
  type DragMode,
  type Span,
  type Zoom,
} from './gantt'
import { usePointerDrag, type DragPoint } from './usePointerDrag'
import { actualSpans, type ActualSpan } from './timeFormat'

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  initiativeById: Map<string, Initiative>
  onSelect: (task: TaskItem) => void
  onReschedule: (task: TaskItem, patch: UpdateTaskInput) => void
  /** Recent work leases, drawn as thin bars under each task's plan. */
  sessions?: WorkSession[]
}

type Line = { kind: 'group'; name: string } | { kind: 'task'; row: TaskRow }

interface DragPayload {
  task: TaskItem
  mode: DragMode | 'place'
}

interface DragState extends DragPayload {
  point: DragPoint
}

const MONDAY = 1
const ARROW_HEAD_SIZE = 6
const TIMELINE_ATTR = 'data-gantt-timeline'
const EDGE_HANDLE_CLASS = 'tl-edge'

export function TaskGanttView({ tasks, allById, initiativeById, onSelect, onReschedule, sessions = [] }: Props) {
  const [zoom, setZoom] = usePersistedState<Zoom>(ZOOM_KEY, 'week', isZoom)
  const [drag, setDrag] = useState<DragState | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const timelineRef = useRef<HTMLDivElement>(null)
  const dayPx = DAY_PX[zoom]
  const today = todayDay()

  const lines = useMemo<Line[]>(() => {
    const out: Line[] = []
    for (const [name, items] of groupByInitiative(tasks, initiativeById)) {
      const rows = nestRows(items, allById).filter((r) => spanOf(r.task) !== null)
      if (rows.length === 0) continue
      out.push({ kind: 'group', name })
      for (const row of rows) out.push({ kind: 'task', row })
    }
    return out
  }, [tasks, allById, initiativeById])

  const actual = useMemo(() => actualSpans(sessions), [sessions])
  const unscheduled = useMemo(() => tasks.filter((t) => spanOf(t) === null), [tasks])
  const range = useMemo(() => (zoom === 'month' ? monthRange(today) : weekRange(today)), [today, zoom])
  const dayCount = range.end - range.start + 1
  const width = dayCount * dayPx

  const dragDelta = drag && drag.mode !== 'place' ? Math.round(drag.point.dx / dayPx) : 0
  const previewSpan = (task: TaskItem): Span | null => {
    const span = spanOf(task)
    if (!span || !drag || drag.task.id !== task.id || drag.mode === 'place') return span
    return applyDrag(span, drag.mode, dragDelta)
  }

  const scrollToToday = () => {
    const el = scrollRef.current
    if (el) el.scrollLeft = (today - range.start) * dayPx - el.clientWidth / 3
  }
  useEffect(scrollToToday, [zoom])

  const dayAtPointer = (point: DragPoint): number | null => {
    const timeline = timelineRef.current
    const target = document.elementFromPoint(point.x, point.y)
    if (!timeline || !target?.closest(`[${TIMELINE_ATTR}]`)) return null
    return range.start + Math.floor((point.x - timeline.getBoundingClientRect().left) / dayPx)
  }

  const startDrag = usePointerDrag<DragPayload>({
    onStart: (payload, point) => setDrag({ ...payload, point }),
    onMove: (payload, point) => setDrag({ ...payload, point }),
    onEnd: (payload, point) => {
      setDrag(null)
      if (payload.mode === 'place') {
        const day = dayAtPointer(point)
        if (day !== null) onReschedule(payload.task, placementPatch(day))
        return
      }
      const delta = Math.round(point.dx / dayPx)
      const span = spanOf(payload.task)
      if (span && delta !== 0) {
        onReschedule(payload.task, spanToPatch(payload.task, applyDrag(span, payload.mode, delta), payload.mode))
      }
    },
    onCancel: () => setDrag(null),
    onClick: (payload) => onSelect(payload.task),
  })

  const rowIndex = new Map<string, number>()
  lines.forEach((line, i) => {
    if (line.kind === 'task') rowIndex.set(line.row.task.id, i)
  })

  return (
    <div className="tl-wrap">
      <GanttToolbar zoom={zoom} onZoom={setZoom} onToday={scrollToToday} />

      <div ref={scrollRef} className="tl">
        <div className="tl-grid">
          <div className="tl-names">
            <div className="tl-head" />
            {lines.map((line) => (
              <GanttLabel key={line.kind === 'group' ? `g-${line.name}` : line.row.task.id} line={line} onSelect={onSelect} />
            ))}
          </div>

          <div className="tl-area" style={{ width }}>
            <GanttHeader rangeStart={range.start} dayCount={dayCount} dayPx={dayPx} zoom={zoom} today={today} />
            <div
              ref={timelineRef}
              {...{ [TIMELINE_ATTR]: true }}
              className="relative"
              style={{ height: Math.max(lines.length, 1) * ROW_HEIGHT }}
            >
              <GanttBackground rangeStart={range.start} dayCount={dayCount} dayPx={dayPx} today={today} zoom={zoom} />
              <DependencyArrows
                lines={lines}
                rowIndex={rowIndex}
                allById={allById}
                previewSpan={previewSpan}
                toX={(day) => (day - range.start) * dayPx}
                width={width}
              />
              {lines.map((line, i) => {
                if (line.kind !== 'task') return null
                const task = line.row.task
                const span = previewSpan(task)
                if (!span) return null
                return (
                  <GanttBar
                    key={task.id}
                    task={task}
                    span={span}
                    top={i * ROW_HEIGHT}
                    left={(span.start - range.start) * dayPx}
                    dayPx={dayPx}
                    overdue={isOverdue(task, today)}
                    dragging={drag?.task.id === task.id}
                    onPointerDown={(e, mode) => startDrag(e, { task, mode })}
                  />
                )
              })}
              {lines.map((line, i) =>
                line.kind === 'task'
                  ? (actual.get(line.row.task.id) ?? []).map((span, n) => {
                      // Work outside the planned window is cut to it, not drawn off the grid.
                      const start = Math.max(span.start, range.start)
                      const end = Math.min(span.end, range.end)
                      if (end < start) return null
                      return (
                        <ActualBar
                          key={`${line.row.task.id}-actual-${n}`}
                          span={{ ...span, start, end }}
                          top={i * ROW_HEIGHT}
                          left={(start - range.start) * dayPx}
                          dayPx={dayPx}
                        />
                      )
                    })
                  : null
              )}
            </div>
          </div>
        </div>
        {lines.length === 0 && (
          <p className="a-grp">{t('timeline_none_scheduled')}</p>
        )}
      </div>

      <UnscheduledTray tasks={unscheduled} draggingId={drag?.task.id} onPointerDown={(e, task) => startDrag(e, { task, mode: 'place' })} />

      {drag?.mode === 'place' && (
        <div
          className="tl-chip bd-ghost"
          style={{ left: drag.point.x + BAR_INSET, top: drag.point.y + BAR_INSET }}
        >
          {drag.task.display_id} {drag.task.title}
        </div>
      )}
    </div>
  )
}

function GanttToolbar({ zoom, onZoom, onToday }: { zoom: Zoom; onZoom: (z: Zoom) => void; onToday: () => void }) {
  return (
    <div className="tk-bar">
      <div className="a-seg">
        {(['week', 'month'] as Zoom[]).map((z) => (
          <button key={z} type="button" aria-pressed={zoom === z} onClick={() => onZoom(z)} className="a-seg-item">
            {t(z === 'week' ? 'timeline_week' : 'timeline_month')}
          </button>
        ))}
      </div>
      <button type="button" onClick={onToday} className="a-btn compact secondary">
        <CalendarDays className="a-ic sm" aria-hidden="true" />
        {t('timeline_this_week')}
      </button>
    </div>
  )
}

function isOverdue(task: TaskItem, today: number): boolean {
  const span = spanOf(task)
  return task.status !== 'complete' && span !== null && span.end < today
}

function GanttLabel({ line, onSelect }: { line: Line; onSelect: (task: TaskItem) => void }) {
  if (line.kind === 'group') {
    return (
      <div className="tl-name is-group" style={{ height: ROW_HEIGHT }}>
        {line.name}
      </div>
    )
  }
  const { task, depth } = line.row
  return (
    <button
      type="button"
      onClick={() => onSelect(task)}
      className="tl-name"
      style={{ height: ROW_HEIGHT, paddingLeft: depth === 1 ? 32 : undefined }}
    >
      <span>{task.display_id}</span>
      <b>{task.title}</b>
    </button>
  )
}

function GanttHeader({ rangeStart, dayCount, dayPx, zoom, today }: { rangeStart: number; dayCount: number; dayPx: number; zoom: Zoom; today: number }) {
  const days = Array.from({ length: dayCount }, (_, i) => rangeStart + i)
  return (
    <div className="tl-days">
      {days.map((day, i) => {
        const monthStart = dayOfMonth(day) === 1 || i === 0
        const showDay = zoom === 'week' || dayOfWeek(day) === MONDAY
        return (
          <div key={day}>
            {monthStart && (
              <span className="tl-day" style={{ left: i * dayPx, border: 0, padding: '2px 4px', width: 'auto' }}>
                {monthLabel(day)}
              </span>
            )}
            {showDay && (
              <span
                className={`tl-day${day === today ? ' today' : ''}${isWeekend(day) ? ' we' : ''}`}
                style={{ left: i * dayPx, width: zoom === 'week' ? dayPx : undefined, top: 'auto', height: 28 }}
              >
                {zoom === 'week' ? dayOfMonth(day) : t('timeline_monday', { day: dayOfMonth(day) })}
              </span>
            )}
          </div>
        )
      })}
    </div>
  )
}

function GanttBackground({ rangeStart, dayCount, dayPx, today, zoom }: { rangeStart: number; dayCount: number; dayPx: number; today: number; zoom: Zoom }) {
  // At Month a day is a few pixels wide, so shading weekends only adds noise.
  const weekends = zoom === 'month' ? [] : Array.from({ length: dayCount }, (_, i) => rangeStart + i).filter(isWeekend)
  return (
    <div className="tl-bg">
      {weekends.map((day) => (
        <div key={day} className="tl-shade" style={{ left: (day - rangeStart) * dayPx, width: dayPx }} />
      ))}
      <div className="tl-now" style={{ left: (today - rangeStart) * dayPx + dayPx / 2 }} />
    </div>
  )
}

interface BarProps {
  task: TaskItem
  span: Span
  top: number
  left: number
  dayPx: number
  overdue: boolean
  dragging: boolean
  onPointerDown: (e: React.PointerEvent, mode: DragMode) => void
}

function barState(task: TaskItem, overdue: boolean): { cls: string; label: string } {
  if (overdue) return { cls: 'late', label: t('tasks_bar_late') }
  switch (task.status) {
    case 'in_progress':
      return { cls: 'work', label: t('tasks_bar_working') }
    case 'ready_for_review':
      return { cls: 'needs', label: STATUS_LABELS.ready_for_review }
    case 'blocked':
      return { cls: 'needs', label: STATUS_LABELS.blocked }
    case 'complete':
      return { cls: 'done', label: STATUS_LABELS.complete }
    default:
      return { cls: '', label: STATUS_LABELS.to_do }
  }
}

function GanttBar({ task, span, top, left, dayPx, overdue, dragging, onPointerDown }: BarProps) {
  const state = barState(task, overdue)
  const title = `${task.display_id} ${task.title}`
  if (span.milestone) {
    return (
      <div
        onPointerDown={(e) => onPointerDown(e, 'move')}
        title={`${title}, ${state.label}`}
        className={`tl-bar milestone ${state.cls}${dragging ? ' is-dragging' : ''}`}
        style={{
          top: top + (ROW_HEIGHT - MILESTONE_SIZE) / 2,
          left: left + (dayPx - MILESTONE_SIZE) / 2,
          width: MILESTONE_SIZE,
          height: MILESTONE_SIZE,
          minHeight: 0,
        }}
      />
    )
  }
  return (
    <div
      onPointerDown={(e) => onPointerDown(e, 'move')}
      title={title}
      className={`tl-bar ${state.cls}${dragging ? ' is-dragging' : ''}`}
      style={{ top: top + BAR_INSET, left, width: (span.end - span.start + 1) * dayPx, height: ROW_HEIGHT - BAR_INSET * 2 }}
    >
      {overdue && <TriangleAlert className="a-ic xs" aria-hidden="true" />}
      <span>{task.title}</span>
      <em>{state.label}</em>
      <div onPointerDown={(e) => onPointerDown(e, 'start')} className={`${EDGE_HANDLE_CLASS}`} style={{ left: 0 }} />
      <div onPointerDown={(e) => onPointerDown(e, 'end')} className={`${EDGE_HANDLE_CLASS}`} style={{ right: 0 }} />
    </div>
  )
}

const ACTUAL_BAR_HEIGHT = 3
const ACTUAL_BAR_OFFSET = 4

/** Where work actually happened, under the plan. A live session is drawn solid. */
function ActualBar({ span, top, left, dayPx }: { span: ActualSpan; top: number; left: number; dayPx: number }) {
  return (
    <div
      aria-hidden
      title={span.live ? t('timeline_working_now') : t('timeline_worked_on')}
      className={`tl-actual${span.live ? ' live' : ''}`}
      style={{
        top: top + ROW_HEIGHT - ACTUAL_BAR_HEIGHT - ACTUAL_BAR_OFFSET,
        left,
        width: Math.max((span.end - span.start + 1) * dayPx, ACTUAL_BAR_HEIGHT * 2),
        height: ACTUAL_BAR_HEIGHT,
      }}
    />
  )
}

interface ArrowProps {
  lines: Line[]
  rowIndex: Map<string, number>
  allById: Map<string, TaskItem>
  previewSpan: (task: TaskItem) => Span | null
  toX: (day: number) => number
  width: number
}

function DependencyArrows({ lines, rowIndex, allById, previewSpan, toX, width }: ArrowProps) {
  const arrows: { key: string; d: string; conflict: boolean }[] = []
  for (const line of lines) {
    if (line.kind !== 'task') continue
    const task = line.row.task
    const span = previewSpan(task)
    const row = rowIndex.get(task.id)
    for (const blockerId of task.depends_on) {
      const blocker = allById.get(blockerId)
      const blockerRow = rowIndex.get(blockerId)
      const blockerSpan = blocker ? previewSpan(blocker) : null
      if (!span || row === undefined || blockerRow === undefined || !blockerSpan) continue
      const center = (r: number) => r * ROW_HEIGHT + ROW_HEIGHT / 2
      arrows.push({
        key: `${blockerId}-${task.id}`,
        d: arrowPath(toX(blockerSpan.end + 1), center(blockerRow), toX(span.start), center(row)),
        conflict: span.start <= blockerSpan.end,
      })
    }
  }
  if (arrows.length === 0) return null
  return (
    <svg className="tl-arrows" width={width} height={lines.length * ROW_HEIGHT}>
      <defs>
        {(['ok', 'conflict'] as const).map((kind) => (
          <marker key={kind} id={`gantt-arrow-${kind}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth={ARROW_HEAD_SIZE} markerHeight={ARROW_HEAD_SIZE} orient="auto">
            <path d="M0,0 L8,4 L0,8 z" fill={kind === 'ok' ? 'var(--ink-muted)' : 'var(--warn)'} />
          </marker>
        ))}
      </defs>
      {arrows.map((a) => (
        <path
          key={a.key}
          d={a.d}
          fill="none"
          strokeWidth={1.5}
          stroke={a.conflict ? 'var(--warn)' : 'var(--ink-muted)'}
          markerEnd={`url(#gantt-arrow-${a.conflict ? 'conflict' : 'ok'})`}
        />
      ))}
    </svg>
  )
}

interface TrayProps {
  tasks: TaskItem[]
  draggingId?: string
  onPointerDown: (e: React.PointerEvent, task: TaskItem) => void
}

function UnscheduledTray({ tasks, draggingId, onPointerDown }: TrayProps) {
  if (tasks.length === 0) return null
  return (
    <div className="tl-tray">
      <h3 className="a-grp">{t('timeline_unscheduled', { n: tasks.length })}</h3>
      <div className="tk-meta">
        {tasks.map((task) => (
          <div
            key={task.id}
            onPointerDown={(e) => onPointerDown(e, task)}
            className={`tl-chip${draggingId === task.id ? ' is-dragging' : ''}`}
          >
            <span>{task.display_id}</span>
            {task.title}
          </div>
        ))}
      </div>
    </div>
  )
}
