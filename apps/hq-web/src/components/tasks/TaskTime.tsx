import { useEffect } from 'react'
import { Loader2 } from 'lucide-react'
import { t } from '~/strings'
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
import './leaf.css'

const TIME_POLL_MS = 120_000
const SESSIONS_SHOWN = 5
const STATUS_LABEL: Record<string, string> = {
  to_do: t('board_col_todo'),
  in_progress: t('board_col_working'),
  blocked: t('board_col_stuck'),
  ready_for_review: t('board_col_ready'),
}

function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="a-drow l-tall l-col">
      <div className="a-label">{label}</div>
      <div className="a-body-sm l-break">{value}</div>
      {note && <div className="a-cap a-muted">{note}</div>}
    </div>
  )
}

function Summary({ time }: { time: TimeSummary }) {
  const estimate = time.estimate_minutes
  const variance = time.variance_minutes
  return (
    <div className="a-card l-lines">
      <div className="l-grid">
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
        <div className="l-wrap a-cap a-muted l-pad">
          {Object.entries(time.status_seconds).map(([status, seconds]) => (
            <span key={status}>
              {STATUS_LABEL[status] ?? status} {formatDuration(seconds)}
            </span>
          ))}
        </div>
      ) : (
        <p className="a-cap a-muted l-pad">Time per status is unknown: this task predates the full event log.</p>
      )}
      {time.subtasks && (
        <p className="a-cap a-muted l-pad">
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
    <li className="a-drow l-tall l-col">
      <div className="l-row justify-between a-body-sm">
        <span className="l-clip">{who}</span>
        <span className="shrink-0 a-muted">{formatDuration(session.active_seconds)}</span>
      </div>
      <div className="a-cap a-muted l-break">
        {formatLocalTime(session.started_at)}
        {session.ended_at === null ? ' · working' : session.end_reason ? ` · ${session.end_reason.replace('_', ' ')}` : ''}
        {session.branch ? ` · ${session.branch}` : ''}
      </div>
    </li>
  )
}

/** Time on a task: planned against worked, where the time went, and which sessions did it. */
export function TaskTime({ task }: { task: TaskItem }) {
  const time = usePolled(task.id, () => fetchTaskTimeClient(task.id), TIME_POLL_MS, true, true, ['task:sync'])
  const sessions = usePolled(`${task.id}|sessions`, async () => (await fetchTaskWorkSessionsClient(task.id)).work_sessions, TIME_POLL_MS, true, true, ['task:sync'])
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
        <div role="status" className="a-loading">
          <Loader2 className="a-ic xs animate-spin" aria-hidden="true" />
          Loading time
        </div>
      ) : !time.data ? (
        <p role="alert" className="a-body-sm a-danger-t">
          {time.error ?? 'Could not load time.'}
        </p>
      ) : (
        <Summary time={time.data} />
      )}
      {sessions.data && sessions.data.length > 0 && (
        <ul className="a-card l-lines mt-3">
          {sessions.data.slice(0, SESSIONS_SHOWN).map((s) => (
            <SessionRow key={s.id} session={s} />
          ))}
          {sessions.data.length > SESSIONS_SHOWN && (
            <li className="a-cap a-muted l-pad">{sessions.data.length - SESSIONS_SHOWN} earlier sessions</li>
          )}
        </ul>
      )}
    </section>
  )
}
