import { TriangleAlert, Hourglass, Repeat } from 'lucide-react'
import './tasks.css'
import type { TaskItem, TaskStatus } from '~/lib/tasksApi'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { parseDay, todayDay } from './dates'
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
  /** Ticks the task done from the row's own box. */
  onToggleDone?: (task: TaskItem) => void
  /** Opens the check for a task that waits on the owner; left out where the edition does not check. */
  onCheck?: (task: TaskItem) => void
  isSelected?: boolean
}

export const STATUS_BADGE_CLASS: Record<TaskStatus, string> = {
  to_do: 'hq-tone-grey',
  in_progress: 'bg-transparent hq-tone-work',
  blocked: 'bg-transparent hq-tone-warn',
  ready_for_review: 'bg-transparent hq-tone-ink',
  complete: 'hq-tone-grey',
}

export const BLOCKED_CHIP_CLASS = 'bg-transparent hq-tone-warn'

/** "2 days late" for an open task past its due day, "due today" on the day, otherwise nothing. */
export function lateLabel(task: Pick<TaskItem, 'due_date' | 'status'>, today: number = todayDay()): { text: string; late: boolean } | null {
  const due = parseDay(task.due_date)
  if (due === null || task.status === 'complete') return null
  if (due === today) return { text: t('task_due_today'), late: false }
  return due < today ? { text: tn('task_late', today - due), late: true } : null
}

function formatTime(iso: string) {
  return relTime(parseSqliteUtc(iso)) || iso
}

const DAY_MS = 86_400_000

function idleDays(task: TaskItem): number {
  return Math.max(1, Math.floor((Date.now() - parseSqliteUtc(task.updated_at).getTime()) / DAY_MS))
}

export function TaskCard({ task, onSelect, parentLabel, compact = false, select, asLink = true, onToggleDone, onCheck, isSelected = false }: Props) {
  const isDone = task.status === 'complete'
  const stale = useIsStale(task.id)
  const lateness = lateLabel(task)
  const workingNow = useWorkingNow(task.id)
  const open = select ? () => select.onToggle(task) : onSelect ? () => onSelect(task) : undefined
  const checked = select ? select.ticked : isDone
  const toggle = select ? () => select.onToggle(task) : onToggleDone ? () => onToggleDone(task) : undefined
  const progress = task.subtask_count > 0 ? Math.round((task.subtask_done / task.subtask_count) * 100) : null

  const meta: string[] = [task.display_id]
  if (!compact) meta.push(STATUS_LABELS[task.status])
  if (parentLabel) meta.push(parentLabel)
  if (task.status === 'blocked' && task.blocked_reason) meta.push(task.blocked_reason)
  if (task.blocked_by.length > 0) meta.push(t('task_waiting_on', { ids: task.blocked_by.join(', ') }))
  if (task.assignees.length > 0) meta.push(t('task_for', { who: task.assignees.join(', ') }))
  if (workingNow) meta.push(t('task_is_working', { who: workingNow }))
  if (task.due_date) meta.push(task.due_date)
  if (task.tags.length > 0) meta.push(task.tags.join(', '))
  meta.push(formatTime(task.updated_at))

  const onLink = (e: React.MouseEvent) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
    e.preventDefault()
    open?.()
  }
  const Main = asLink && !select ? 'a' : 'button'
  const mainProps = Main === 'a' ? { href: `/tasks/${encodeURIComponent(task.id)}`, onClick: onLink } : { type: 'button' as const, onClick: open }

  return (
    <div className={`a-trow${isSelected || select?.ticked ? ' is-sel' : ''}${isDone ? ' is-done' : ''}`} data-task-id={task.id}>
      <button
        type="button"
        role="checkbox"
        aria-checked={checked}
        aria-label={select ? t('task_select', { id: task.display_id }) : t('tasks_mark_done')}
        className="a-chk"
        data-hit="select"
        onClick={toggle}
      >
        <u />
      </button>
      <Main {...mainProps} className="a-trow-main">
        <span className="a-trow-title">{task.title}</span>
        <span className="a-trow-meta">
          {meta.join(' · ')}
          {lateness && (
            <>
              {' · '}
              <b>
                {lateness.late && <TriangleAlert className="a-ic xs" aria-hidden="true" />}
                {lateness.text}
              </b>
            </>
          )}
        </span>
        {(stale || task.long_horizon) && (
          <span className="tk-meta">
            {stale && (
              <span className="a-kind sm" title={t('task_stale_hint')}>
                <Hourglass className="a-ic xs" aria-hidden="true" />
                {tn('task_no_change', idleDays(task))}
              </span>
            )}
            {task.long_horizon && (
              <span className="a-kind sm" title={t('task_long_running_hint')}>
                <Repeat className="a-ic xs" aria-hidden="true" />
                {t('task_long_running')}
              </span>
            )}
          </span>
        )}
        {progress !== null && (
          <>
            <span className="a-prog" aria-hidden="true">
              <u style={{ width: `${progress}%` }} />
            </span>
            <span className="a-trow-meta">{t('task_subtasks_done', { done: task.subtask_done, total: task.subtask_count })}</span>
          </>
        )}
      </Main>
      {onCheck && task.status === 'ready_for_review' && !select && (
        <button type="button" className="a-btn compact secondary a-trow-act" onClick={() => onCheck(task)}>
          {t('tasks_check_it')}
        </button>
      )}
    </div>
  )
}
