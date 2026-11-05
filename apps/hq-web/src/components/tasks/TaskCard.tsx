import { Clock, TriangleAlert, Tag, Calendar, ListChecks, Link2, CornerDownRight } from 'lucide-react'
import type { TaskItem, TaskPriority, TaskStatus } from '~/lib/tasksApi'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { parseDay, shortLabel, todayDay } from './dates'
import { t, tn } from '~/strings'
import { useIsStale } from './staleContext'
import { useWorkingNow } from './workingContext'

interface Props {
  task: TaskItem
  /** Omit when a parent handles selection itself (e.g. the board's drag hook). */
  onSelect?: (task: TaskItem) => void
  /** Shown when a sub-task appears without its parent next to it. */
  parentLabel?: string
  compact?: boolean
  /** Set while picking tasks for a bulk action: a click ticks the card instead of opening it. */
  select?: { ticked: boolean; onToggle: (task: TaskItem) => void }
  /** The whole card is a link to the task, so it opens in a new tab and works from the keyboard. */
  asLink?: boolean
}

export const STATUS_BADGE_CLASS: Record<TaskStatus, string> = {
  to_do: 'bg-white/5 hq-tone-grey',
  in_progress: 'bg-transparent hq-tone-work',
  blocked: 'bg-transparent hq-tone-warn',
  ready_for_review: 'bg-transparent hq-tone-ink',
  complete: 'bg-white/5 hq-tone-grey',
}

export const BLOCKED_CHIP_CLASS = 'bg-transparent hq-tone-warn'

const CHIP_CLASS = 'inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full border max-w-full'

/** "2 days late" for an open task past its due day, "due today" on the day, otherwise nothing. */
export function lateLabel(task: Pick<TaskItem, 'due_date' | 'status'>, today: number = todayDay()): { text: string; late: boolean } | null {
  const due = parseDay(task.due_date)
  if (due === null || task.status === 'complete') return null
  if (due === today) return { text: t('task_due_today'), late: false }
  return due < today ? { text: tn('task_late', today - due), late: true } : null
}

function scheduleLabel(task: TaskItem): string | null {
  if (task.start_date && task.due_date) return `${shortLabel(task.start_date)} → ${shortLabel(task.due_date)}`
  if (task.due_date) return task.due_date
  if (task.start_date) return `from ${shortLabel(task.start_date)}`
  return null
}

const PRIORITY_DOT_CLASS: Record<TaskPriority, string> = {
  urgent: 'bg-[var(--warn)]',
  high: 'bg-[var(--ink-muted)]',
  normal: 'bg-neutral-500',
  low: 'bg-neutral-700',
}

function formatTime(iso: string) {
  return relTime(parseSqliteUtc(iso)) || iso
}

const DAY_MS = 86_400_000

function idleDays(task: TaskItem): number {
  return Math.max(1, Math.floor((Date.now() - parseSqliteUtc(task.updated_at).getTime()) / DAY_MS))
}

export function TaskCard({ task, onSelect, parentLabel, compact = false, select, asLink = false }: Props) {
  const isDone = task.status === 'complete'
  const schedule = scheduleLabel(task)
  const stale = useIsStale(task.id)
  const lateness = lateLabel(task)
  const workingNow = useWorkingNow(task.id)
  const hasChips =
    task.tags.length > 0 ||
    task.assignees.length > 0 ||
    schedule ||
    lateness ||
    task.subtask_count > 0 ||
    task.blocked_by.length > 0 ||
    stale ||
    Boolean(workingNow) ||
    task.long_horizon ||
    Boolean(task.blocked_reason)

  const onClick = select ? () => select.onToggle(task) : onSelect ? () => onSelect(task) : undefined

  const Root = asLink ? 'a' : 'div'
  const rootProps = asLink
    ? {
        href: `/tasks/${encodeURIComponent(task.id)}`,
        onClick: (e: React.MouseEvent) => {
          if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
          e.preventDefault()
          onClick?.()
        },
      }
    : { onClick }

  return (
    <Root
      {...rootProps}
      className={`group relative block rounded-2xl border transition-all duration-200 cursor-pointer w-full max-w-full min-w-0 overflow-hidden ${compact ? 'p-3' : 'p-4'} ${
        isDone
          ? 'hq-card opacity-60 hover:opacity-100'
          : 'hq-card hq-card-live'
      } ${select?.ticked ? 'ring-1 ring-white/60' : ''}`}
    >
      <div className="flex items-center justify-between gap-2 flex-wrap mb-2">
        <div className="flex items-center gap-2 min-w-0 flex-wrap">
          {select && (
            // The card's own click does the toggling; the box only shows the state (and takes the keyboard).
            <span className="inline-flex items-center justify-center w-12 h-12 -m-4 shrink-0" data-hit="select">
              <input
                type="checkbox"
                checked={select.ticked}
                onChange={() => {}}
                aria-label={t('task_select', { id: task.display_id })}
                className="w-5 h-5 cursor-pointer accent-white"
              />
            </span>
          )}
          {task.priority && (
            <span
              className={`w-1.5 h-1.5 rounded-full shrink-0 ${PRIORITY_DOT_CLASS[task.priority]}`}
              title={t('task_priority_hint', { priority: task.priority })}
            />
          )}
          <span className="text-xs font-bold text-neutral-500 shrink-0">{task.display_id}</span>
          {!compact && (
            <span
              className={`text-xs font-bold uppercase tracking-wider px-2 py-0.5 rounded-full border shrink-0 ${STATUS_BADGE_CLASS[task.status]}`}
            >
              {STATUS_LABELS[task.status]}
            </span>
          )}
        </div>
        {!compact && (
          <span className="text-xs text-neutral-500 flex items-center gap-1 shrink-0 ml-auto sm:ml-0">
            <Clock className="w-3 h-3 shrink-0" />
            <span>{formatTime(task.updated_at)}</span>
          </span>
        )}
      </div>

      {parentLabel && (
        <p className="text-xs text-neutral-500 mb-1 flex items-center gap-1 min-w-0">
          <CornerDownRight className="w-3 h-3 shrink-0" />
          <span className="truncate">{parentLabel}</span>
        </p>
      )}
      <h3 className="text-sm font-semibold text-white group-hover:underline transition-colors break-words">
        {task.title}
      </h3>
      {task.description && !compact && (
        <p className="text-xs text-neutral-400 mt-1 line-clamp-2 leading-relaxed break-words">{task.description}</p>
      )}

      {hasChips && (
        <div className="flex items-center gap-2 flex-wrap mt-3 min-w-0">
          {task.status === 'blocked' && task.blocked_reason && (
            <span className={`${CHIP_CLASS} ${BLOCKED_CHIP_CLASS}`} title={task.waiting_on ? t('task_waiting_on', { ids: task.waiting_on }) : undefined}>
              <span className="truncate">{task.blocked_reason}</span>
            </span>
          )}
          {task.assignees.length > 0 && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-200 border-white/10 max-w-[240px]`} title={t('task_assigned_to')}>
              <span className="truncate">{t('task_for', { who: task.assignees.join(', ') })}</span>
            </span>
          )}
          {workingNow && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-100 border-white/10 max-w-[220px]`} title={t('task_work_lease_hint')}>
              <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse shrink-0" />
              <span className="truncate">{t('task_is_working', { who: workingNow })}</span>
            </span>
          )}
          {stale && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 shrink-0`} title={t('task_stale_hint')}>
              <Clock className="w-2.5 h-2.5 shrink-0" />
              <span>{tn('task_no_change', idleDays(task))}</span>
            </span>
          )}
          {task.long_horizon && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-400 border-white/10 shrink-0`} title={t('task_long_running_hint')}>
              <span>{t('task_long_running')}</span>
            </span>
          )}
          {task.blocked_by.length > 0 && (
            <span className={`${CHIP_CLASS} ${BLOCKED_CHIP_CLASS}`} title={t('task_waiting_on', { ids: task.blocked_by.join(', ') })}>
              <Link2 className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">{t('task_waiting_on', { ids: task.blocked_by.join(', ') })}</span>
            </span>
          )}
          {task.subtask_count > 0 && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 shrink-0`} title={t('task_subtasks_done', { done: task.subtask_done, total: task.subtask_count })}>
              <ListChecks className="w-2.5 h-2.5 shrink-0" />
              <span>{t('task_subtasks_done', { done: task.subtask_done, total: task.subtask_count })}</span>
            </span>
          )}
          {task.tags.map((tag) => (
            <span key={tag} className={`${CHIP_CLASS} bg-white/5 text-neutral-300 border-white/10 max-w-[200px]`}>
              <Tag className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">{tag}</span>
            </span>
          ))}
          {lateness && (
            <span className={`${CHIP_CLASS} bg-transparent ${lateness.late ? 'hq-tone-warn' : 'hq-tone-grey'} shrink-0`}>
              {lateness.late && <TriangleAlert className="w-3 h-3 shrink-0" aria-hidden="true" />}
              <span>{lateness.text}</span>
            </span>
          )}
          {schedule && (
            <span className={`${CHIP_CLASS} bg-white/5 text-neutral-400 border-white/10 shrink-0`}>
              <Calendar className="w-2.5 h-2.5 shrink-0" />
              <span className="truncate">{schedule}</span>
            </span>
          )}
        </div>
      )}
    </Root>
  )
}
