import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarDays } from 'lucide-react'
import type { Initiative, TaskItem, UpdateTaskInput } from '~/lib/tasksApi'
import { BLOCKED_CHIP_CLASS, STATUS_BADGE_CLASS } from './TaskCard'
import { groupByInitiative, nestRows, type TaskRow } from './hierarchy'
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
  visibleRange,
  type DragMode,
  type Span,
  type Zoom,
} from './gantt'
import { usePointerDrag, type DragPoint } from './usePointerDrag'

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  initiativeById: Map<string, Initiative>
  onSelect: (task: TaskItem) => void
  onReschedule: (task: TaskItem, patch: UpdateTaskInput) => void
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
const EDGE_HANDLE_CLASS = 'absolute top-0 bottom-0 w-2 cursor-ew-resize'

export function TaskGanttView({ tasks, allById, initiativeById, onSelect, onReschedule }: Props) {
  const [zoom, setZoom] = useState<Zoom>('week')
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

  const unscheduled = useMemo(() => tasks.filter((t) => spanOf(t) === null), [tasks])
  const range = useMemo(
    () => visibleRange(tasks.map(spanOf).filter((s): s is Span => s !== null), today),
    [tasks, today]
  )
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
    <div className="pb-6 space-y-3">
      <GanttToolbar zoom={zoom} onZoom={setZoom} onToday={scrollToToday} />

      <div ref={scrollRef} className="relative overflow-auto rounded-2xl border border-white/10 bg-black/20 max-h-[70vh]">
        <div className="flex min-w-max">
          <div className="sticky left-0 z-20 w-36 sm:w-56 shrink-0 border-r border-white/10 bg-neutral-950">
            <div className="sticky top-0 z-10 h-11 border-b border-white/10 bg-neutral-950" />
            {lines.map((line) => (
              <GanttLabel key={line.kind === 'group' ? `g-${line.name}` : line.row.task.id} line={line} onSelect={onSelect} />
            ))}
          </div>

          <div className="relative" style={{ width }}>
            <GanttHeader rangeStart={range.start} dayCount={dayCount} dayPx={dayPx} zoom={zoom} />
            <div
              ref={timelineRef}
              {...{ [TIMELINE_ATTR]: true }}
              className="relative"
              style={{ height: Math.max(lines.length, 1) * ROW_HEIGHT }}
            >
              <GanttBackground rangeStart={range.start} dayCount={dayCount} dayPx={dayPx} today={today} />
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
            </div>
          </div>
        </div>
        {lines.length === 0 && (
          <p className="px-4 py-10 text-center text-xs font-mono text-neutral-500">
            No scheduled tasks. Drag one from the tray below onto the timeline.
          </p>
        )}
      </div>

      <UnscheduledTray tasks={unscheduled} draggingId={drag?.task.id} onPointerDown={(e, task) => startDrag(e, { task, mode: 'place' })} />

      {drag?.mode === 'place' && (
        <div
          className="fixed z-50 pointer-events-none px-2.5 py-1 rounded-lg text-[11px] font-mono border shadow-2xl bg-neutral-950 text-neutral-200 border-emerald-500/40"
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
    <div className="flex items-center gap-2">
      <div className="flex items-center gap-1 p-1 rounded-xl bg-black/40 border border-white/10">
        {(['week', 'month'] as Zoom[]).map((z) => (
          <button
            key={z}
            type="button"
            onClick={() => onZoom(z)}
            className={`px-3 py-1 rounded-lg text-xs font-mono font-semibold capitalize transition-all ${
              zoom === z ? 'bg-white/10 text-emerald-400' : 'text-neutral-400 hover:text-neutral-200'
            }`}
          >
            {z}
          </button>
        ))}
      </div>
      <button
        type="button"
        onClick={onToday}
        className="px-3 py-1.5 rounded-xl border border-white/10 text-xs font-mono text-neutral-300 hover:text-white hover:bg-white/5 flex items-center gap-1.5"
      >
        <CalendarDays className="w-3.5 h-3.5" />
        Today
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
      <div
        className="px-3 flex items-end pb-1 text-[10px] font-mono font-bold uppercase tracking-wider text-neutral-500 truncate"
        style={{ height: ROW_HEIGHT }}
      >
        {line.name}
      </div>
    )
  }
  const { task, depth } = line.row
  return (
    <button
      type="button"
      onClick={() => onSelect(task)}
      className={`w-full text-left px-3 flex items-center gap-1.5 hover:bg-white/5 ${depth === 1 ? 'pl-7' : ''}`}
      style={{ height: ROW_HEIGHT }}
    >
      <span className="text-[10px] font-mono text-neutral-500 shrink-0">{task.display_id}</span>
      <span className={`text-xs truncate ${task.status === 'complete' ? 'text-neutral-500 line-through' : 'text-neutral-200'}`}>
        {task.title}
      </span>
    </button>
  )
}

function GanttHeader({ rangeStart, dayCount, dayPx, zoom }: { rangeStart: number; dayCount: number; dayPx: number; zoom: Zoom }) {
  const days = Array.from({ length: dayCount }, (_, i) => rangeStart + i)
  return (
    <div className="sticky top-0 z-10 h-11 border-b border-white/10 bg-neutral-950">
      {days.map((day, i) => {
        const monthStart = dayOfMonth(day) === 1 || i === 0
        const showDay = zoom === 'week' || dayOfWeek(day) === MONDAY
        return (
          <div key={day}>
            {monthStart && (
              <span className="absolute top-1 text-[10px] font-mono font-semibold text-neutral-400 pl-1 whitespace-nowrap" style={{ left: i * dayPx }}>
                {monthLabel(day)}
              </span>
            )}
            {showDay && (
              <span
                className="absolute bottom-1 text-[10px] font-mono text-neutral-500 text-center"
                style={{ left: i * dayPx, width: zoom === 'week' ? dayPx : undefined }}
              >
                {dayOfMonth(day)}
              </span>
            )}
          </div>
        )
      })}
    </div>
  )
}

function GanttBackground({ rangeStart, dayCount, dayPx, today }: { rangeStart: number; dayCount: number; dayPx: number; today: number }) {
  const weekends = Array.from({ length: dayCount }, (_, i) => rangeStart + i).filter(isWeekend)
  return (
    <div className="absolute inset-0 pointer-events-none">
      {weekends.map((day) => (
        <div key={day} className="absolute top-0 bottom-0 bg-white/[0.025]" style={{ left: (day - rangeStart) * dayPx, width: dayPx }} />
      ))}
      <div className="absolute top-0 bottom-0 w-px bg-emerald-400/60" style={{ left: (today - rangeStart) * dayPx + dayPx / 2 }} />
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

function GanttBar({ task, span, top, left, dayPx, overdue, dragging, onPointerDown }: BarProps) {
  const colors = overdue ? BLOCKED_CHIP_CLASS : STATUS_BADGE_CLASS[task.status]
  const title = `${task.display_id} ${task.title}`
  if (span.milestone) {
    return (
      <div
        onPointerDown={(e) => onPointerDown(e, 'move')}
        title={title}
        className={`absolute rotate-45 border cursor-grab select-none [-webkit-touch-callout:none] ${colors} ${dragging ? 'ring-1 ring-emerald-400' : ''}`}
        style={{
          top: top + (ROW_HEIGHT - MILESTONE_SIZE) / 2,
          left: left + (dayPx - MILESTONE_SIZE) / 2,
          width: MILESTONE_SIZE,
          height: MILESTONE_SIZE,
        }}
      />
    )
  }
  return (
    <div
      onPointerDown={(e) => onPointerDown(e, 'move')}
      title={title}
      className={`absolute rounded-md border cursor-grab select-none [-webkit-touch-callout:none] overflow-hidden ${colors} ${dragging ? 'ring-1 ring-emerald-400' : ''}`}
      style={{ top: top + BAR_INSET, left, width: (span.end - span.start + 1) * dayPx, height: ROW_HEIGHT - BAR_INSET * 2 }}
    >
      <span className="absolute inset-0 px-2 flex items-center text-[10px] font-mono whitespace-nowrap">{task.title}</span>
      <div onPointerDown={(e) => onPointerDown(e, 'start')} className={`${EDGE_HANDLE_CLASS} left-0`} />
      <div onPointerDown={(e) => onPointerDown(e, 'end')} className={`${EDGE_HANDLE_CLASS} right-0`} />
    </div>
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
    <svg className="absolute inset-0 pointer-events-none overflow-visible" width={width} height={lines.length * ROW_HEIGHT}>
      <defs>
        {(['ok', 'conflict'] as const).map((kind) => (
          <marker key={kind} id={`gantt-arrow-${kind}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth={ARROW_HEAD_SIZE} markerHeight={ARROW_HEAD_SIZE} orient="auto">
            <path d="M0,0 L8,4 L0,8 z" className={kind === 'ok' ? 'fill-neutral-500' : 'fill-rose-400'} />
          </marker>
        ))}
      </defs>
      {arrows.map((a) => (
        <path
          key={a.key}
          d={a.d}
          fill="none"
          strokeWidth={1.5}
          className={a.conflict ? 'stroke-rose-400' : 'stroke-neutral-500'}
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
    <div className="rounded-2xl border border-dashed border-white/10 p-3">
      <h3 className="text-[11px] font-mono font-bold uppercase tracking-wider text-neutral-500 mb-2">
        Unscheduled ({tasks.length}), drag onto the timeline to schedule
      </h3>
      <div className="flex flex-wrap gap-2">
        {tasks.map((task) => (
          <div
            key={task.id}
            onPointerDown={(e) => onPointerDown(e, task)}
            className={`px-2.5 py-1 rounded-lg text-[11px] font-mono border cursor-grab select-none [-webkit-touch-callout:none] bg-white/5 text-neutral-300 border-white/10 hover:border-emerald-500/30 ${
              draggingId === task.id ? 'opacity-30' : ''
            }`}
          >
            <span className="text-neutral-500 mr-1.5">{task.display_id}</span>
            {task.title}
          </div>
        ))}
      </div>
    </div>
  )
}
