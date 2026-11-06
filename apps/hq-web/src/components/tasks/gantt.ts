import type { TaskItem, UpdateTaskInput } from '~/lib/tasksApi'
import { dayOfWeek, formatDay, parseDay } from './dates'

export type Zoom = 'week' | 'month'
export type DragMode = 'move' | 'start' | 'end'

export const ZOOM_KEY = 'hq.tasks.timeline.zoom'

export function isZoom(v: unknown): v is Zoom {
  return v === 'week' || v === 'month'
}

export const DAY_PX: Record<Zoom, number> = { week: 56, month: 12 }
export const ROW_HEIGHT = 52
export const BAR_INSET = 10
export const MILESTONE_SIZE = 20
/** Duration given to an unscheduled task dropped onto the grid. */
const NEW_TASK_SPAN_DAYS = 3
const ARROW_CURVE_PX = 28

/** Inclusive day range a task occupies. A due date alone is a milestone. */
export interface Span {
  start: number
  end: number
  milestone: boolean
}

export function spanOf(task: TaskItem): Span | null {
  const start = parseDay(task.start_date)
  const due = parseDay(task.due_date)
  if (start === null && due === null) return null
  if (start === null) return { start: due!, end: due!, milestone: true }
  if (due === null) return { start, end: start, milestone: false }
  return { start: Math.min(start, due), end: Math.max(start, due), milestone: false }
}

export function applyDrag(span: Span, mode: DragMode, delta: number): Span {
  if (mode === 'move') return { ...span, start: span.start + delta, end: span.end + delta }
  if (mode === 'start') return { ...span, start: Math.min(span.start + delta, span.end) }
  return { ...span, end: Math.max(span.end + delta, span.start) }
}

/** The PATCH that persists `span`, touching only the dates the task actually uses. */
export function spanToPatch(task: TaskItem, span: Span, mode: DragMode): UpdateTaskInput {
  if (span.milestone) return { due_date: formatDay(span.end) }
  if (!task.due_date && mode === 'move') return { start_date: formatDay(span.start) }
  return { start_date: formatDay(span.start), due_date: formatDay(span.end) }
}

export function placementPatch(day: number): UpdateTaskInput {
  return { start_date: formatDay(day), due_date: formatDay(day + NEW_TASK_SPAN_DAYS - 1) }
}

const MONTH_WEEKS = 5
const WEEK_ZOOM_WEEKS = 2
const DAYS_PER_WEEK = 7

/** Month is always five weeks from this week's Monday, however far the tasks reach. */
export function monthRange(today: number): { start: number; end: number } {
  const start = today - ((dayOfWeek(today) + 6) % DAYS_PER_WEEK)
  return { start, end: start + MONTH_WEEKS * DAYS_PER_WEEK - 1 }
}

/** Week is two weeks from this week's Monday, the 14 days the design shows. */
export function weekRange(today: number): { start: number; end: number } {
  const start = today - ((dayOfWeek(today) + 6) % DAYS_PER_WEEK)
  return { start, end: start + WEEK_ZOOM_WEEKS * DAYS_PER_WEEK - 1 }
}

/** Curved connector from the end of a blocker's bar to the start of the dependent's. */
export function arrowPath(x1: number, y1: number, x2: number, y2: number): string {
  return `M ${x1} ${y1} C ${x1 + ARROW_CURVE_PX} ${y1}, ${x2 - ARROW_CURVE_PX} ${y2}, ${x2} ${y2}`
}
