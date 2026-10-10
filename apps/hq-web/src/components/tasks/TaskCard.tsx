import { Clock, Tag, Calendar, ListChecks, Link2, CornerDownRight } from 'lucide-react'
import type { TaskItem, TaskPriority, TaskStatus } from '~/lib/tasksApi'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { shortLabel } from './dates'
import { useIsStale } from './staleContext'
import { useWorkingNow } from './workingContext'

interface Props {
  task: TaskItem
  /** Omit when a parent handles selection itself (e.g. the board's drag hook). */
  onSelect?: (task: TaskItem) => void
  /** Shown when a sub-task appears without its parent next to it. */
  parentLabel?: string
  compact?: boolean
}

export const STATUS_BADGE_CLASS: Record<TaskStatus, string> = {
  to_do: 'bg-white/5 text-neutral-400 border-white/10',
  in_progress: 'bg-blue-500/10 text-blue-400 border-blue-500/20',
  blocked: 'bg-rose-500/10 text-rose-400 border-rose-500/20',
  ready_for_review: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  complete: 'bg-white/5 text-neutral-500 border-white/5',
}

export const BLOCKED_CHIP_CLASS = 'bg-rose-500/10 text-rose-400 border-rose-500/20'

const CHIP_CLASS = 'inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full border max-w-full'

function scheduleLabel(task: TaskItem): string | null {
  if (task.start_date && task.due_date) return `${shortLabel(task.start_date)} → ${shortLabel(task.due_date)}`
  if (task.due_date) return task.due_date
  if (task.start_date) return `from ${shortLabel(task.start_date)}`
  return null
}

const PRIORITY_DOT_CLASS: Record<TaskPriority, string> = {
  urgent: 'bg-rose-400',
  high: 'bg-amber-400',
  normal: 'bg-neutral-500',
  low: 'bg-neutral-700',
}

function formatTime(iso: string) {
  return relTime(parseSqliteUtc(iso)) || iso
}

export function TaskCard({ task, onSelect, parentLabel, compact = false }: Props) {
  const isDone = task.status === 'complete'
  const schedule = scheduleLabel(task)
  const stale = useIsStale(task.id)
  const workingNow = useWorkingNow(task.id)
  const hasChips =
    task.tags.length > 0 ||
    task.assignees.length > 0 ||
    schedule ||
    task.subtask_count > 0 ||
    task.blocked_by.length > 0 ||
    stale ||
    Boolean(workingNow) ||
    task.long_horizon ||
    Boolean(task.blocked_reason)

  return (
    <div
      onClick={onSelect ? () => onSelect(task) : undefined}
      className={`group relative rounded-2xl border transition-all duration-200 cursor-pointer w-full max-w-full min-w-0 overflow-hidden ${compact ? 'p-3' : 'p-4'} ${
        isDone
          ? 'hq-card opacity-60 hover:opacity-100'
          : 'hq-card hq-card-live'
      }`}
    >
      <div className="flex items-center justify-between gap-2 flex-wrap mb-2">
        <div className="flex items-center gap-2 min-w-0 flex-wrap">
          {task.priority && (
            <span
              className={`w-1.5 h-1.5 rounded-full shrink-0 ${PRIORITY_DOT_CLASS[task.priority]}`}
              title={`Priority: ${task.priority}`}
            />
          )}
          <span className="text-[11px] font-bold text-neutral-500 shrink-0">{task.display_id}</span>
          {!compact && (
            <span
              className={`text-[11px] font-bold uppercase tracking-wider px-2 py-0.5 rounded-full border shrink-0 ${STATUS_BADGE_CLASS[task.status]}`}
            >
              {STATUS_LABELS[task.status]}
            </span>
          )}
        </div>
        {!compact && (
          <span className="text-[11px] text-neutral-500 flex items-center gap-1 shrink-0 ml-auto sm:ml-0">
            <Clock className="w-3 h-3 shrink-0" />
            <span>{formatTime(task.updated_at)}</span>
          </span>
        )}
      </div>

      {parentLabel && (
        <p className="text-[11px] text-neutral-500 mb-1 flex items-center gap-1 min-w-0">
          <CornerDownRight className="w-3 h-3 shrink-0" />
          <span className="truncate">{parentLabel}</span>
        </p>
      )}
      <h3 className="text-sm font-semibold text-white group-hover:text-emerald-300 transition-colors break-words">
        {task.title}
      </h3>
      {task.description && !compact && (
        <p className="text-xs text-neutral-400 mt-1 line-clamp-2 leading-relaxed break-words">{task.description}</p>
      )}

      {hasChips && (
        <div className="flex items-center gap-2 flex-wrap mt-3 min-w-0">
          {task.status === 'blocked' && task.blocked_reason && (
            <span className={`${CHIP_CLASS} ${BLOCKED_CHIP_CLASS}`} title={task.waiting_on ? `Waiting on ${task.waiting_on}` : undefined}>
              <span className="truncate">{task.blocked_reason}</span>
            </span>
          )}
          {task.assignees.length > 0 && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-200 border-white/10 max-w-[240px]`} title="Assigned to">
              <span className="truncate">for {task.assignees.join(', ')}</span>
            </span>
          )}
          {workingNow && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-100 border-white/10 max-w-[220px]`} title="A session holds a work lease on this task">
              <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse shrink-0" />
              <span className="truncate">{workingNow} is working</span>
            </span>
          )}
          {stale && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 shrink-0`} title="In progress, but nobody holds it and nothing has changed for a while">
              <Clock className="w-2.5 h-2.5 shrink-0" />
              <span>stale</span>
            </span>
          )}
          {task.long_horizon && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-400 border-white/10 shrink-0`} title="Long running work: a session ending does not move it">
              <span>long running</span>
            </span>
          )}
          {task.blocked_by.length > 0 && (
            <span className={`${CHIP_CLASS} ${BLOCKED_CHIP_CLASS}`} title={`Waiting on: ${task.blocked_by.join(', ')}`}>
              <Link2 className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">blocked by {task.blocked_by.join(', ')}</span>
            </span>
          )}
          {task.subtask_count > 0 && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 shrink-0`} title="Sub-tasks complete">
              <ListChecks className="w-2.5 h-2.5 shrink-0" />
              <span>{task.subtask_done}/{task.subtask_count}</span>
            </span>
          )}
          {task.tags.map((tag) => (
            <span key={tag} className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 max-w-[200px]`}>
              <Tag className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">{tag}</span>
            </span>
          ))}
          {schedule && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-400 border-white/10 shrink-0`}>
              <Calendar className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">{schedule}</span>
            </span>
          )}
        </div>
      )}
    </div>
  )
}
