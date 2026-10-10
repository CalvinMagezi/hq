import { useEffect } from 'react'
import { Loader2 } from 'lucide-react'
import {
  fetchTaskTimeClient,
  fetchTaskWorkSessionsClient,
  type TaskItem,
  type TimeSummary,
  type WorkSession,
} from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { formatDuration, formatLocalTime, formatSignedMinutes } from './timeFormat'
import { useRefreshOn } from '~/lib/useRefreshOn'

const TIME_POLL_MS = 120_000
const SESSIONS_SHOWN = 5
const STATUS_LABEL: Record<string, string> = {
  to_do: 'To do',
  in_progress: 'In progress',
  blocked: 'Blocked',
  ready_for_review: 'Ready for review',
}

function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[11px] uppercase tracking-wider text-neutral-500">{label}</div>
      <div className="text-sm text-neutral-200 break-words">{value}</div>
      {note && <div className="text-[11px] text-neutral-500">{note}</div>}
    </div>
  )
}

function Summary({ time }: { time: TimeSummary }) {
  const estimate = time.estimate_minutes
  const variance = time.variance_minutes
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <Stat
          label="Worked"
          value={time.lease_count === 0 ? 'none yet' : formatDuration(time.leased_seconds)}
          note={time.live ? 'a session is working now' : undefined}
        />
        <Stat
          label="Estimate"
          value={estimate === null ? 'none' : formatDuration(estimate * 60)}
          note={variance === null ? undefined : `${formatSignedMinutes(variance)} against it`}
        />
        <Stat label="Time to start" value={formatDuration(time.time_to_start_seconds)} />
        <Stat label="Cycle time" value={time.cycle_seconds === null ? 'not complete' : formatDuration(time.cycle_seconds)} />
      </div>
      {time.status_seconds ? (
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-neutral-400">
          {Object.entries(time.status_seconds).map(([status, seconds]) => (
            <span key={status}>
              {STATUS_LABEL[status] ?? status} {formatDuration(seconds)}
            </span>
          ))}
        </div>
      ) : (
        <p className="text-[11px] text-neutral-500">Time per status is unknown: this task predates the full event log.</p>
      )}
      {time.subtasks && (
        <p className="text-[11px] text-neutral-400">
          Sub-tasks: {formatDuration(time.subtasks.leased_seconds)} worked
          {time.subtasks.with_estimate > 0
            ? `, ${formatDuration(time.subtasks.estimate_minutes * 60)} estimated across ${time.subtasks.with_estimate} of ${time.subtasks.count}`
            : `, none of ${time.subtasks.count} estimated`}
        </p>
      )}
    </div>
  )
}

function SessionRow({ session }: { session: WorkSession }) {
  const who = [session.actor, session.harness].filter(Boolean).join(' · ')
  return (
    <li className="px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5 space-y-0.5">
      <div className="flex items-center justify-between gap-2 text-xs text-neutral-200">
        <span className="truncate">{who}</span>
        <span className="shrink-0 text-neutral-400">{formatDuration(session.active_seconds)}</span>
      </div>
      <div className="text-[11px] text-neutral-500 break-words">
        {formatLocalTime(session.started_at)}
        {session.ended_at === null ? ' · working' : session.end_reason ? ` · ${session.end_reason.replace('_', ' ')}` : ''}
        {session.branch ? ` · ${session.branch}` : ''}
      </div>
    </li>
  )
}

/** Time on a task: planned against worked, where the time went, and which sessions did it. */
export function TaskTime({ task }: { task: TaskItem }) {
  const time = usePolled(task.id, () => fetchTaskTimeClient(task.id), TIME_POLL_MS)
  const sessions = usePolled(`${task.id}|sessions`, async () => (await fetchTaskWorkSessionsClient(task.id)).work_sessions, TIME_POLL_MS)
  useRefreshOn(['task:sync'], time.refresh)
  useRefreshOn(['task:sync'], sessions.refresh)
  const { refresh: refreshTime } = time
  const { refresh: refreshSessions } = sessions
  // updated_at changes with every write: ask again without dropping what is on screen.
  useEffect(() => {
    void refreshTime()
    void refreshSessions()
  }, [task.updated_at, refreshTime, refreshSessions])
  return (
    <section aria-label="Time on task">
      <SectionLabel>Time</SectionLabel>
      {time.loading ? (
        <div role="status" className="flex items-center gap-2 text-xs text-neutral-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading time
        </div>
      ) : !time.data ? (
        <p role="alert" className="text-xs text-rose-400">
          {time.error ?? 'Could not load time.'}
        </p>
      ) : (
        <Summary time={time.data} />
      )}
      {sessions.data && sessions.data.length > 0 && (
        <ul className="mt-3 space-y-1.5">
          {sessions.data.slice(0, SESSIONS_SHOWN).map((s) => (
            <SessionRow key={s.id} session={s} />
          ))}
          {sessions.data.length > SESSIONS_SHOWN && (
            <li className="text-[11px] text-neutral-500">{sessions.data.length - SESSIONS_SHOWN} earlier sessions</li>
          )}
        </ul>
      )}
    </section>
  )
}
