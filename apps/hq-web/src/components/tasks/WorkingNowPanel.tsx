import { useEffect, useState } from 'react'
import { agentName } from '~/lib/names'
import { parseSqliteUtc, type TaskItem, type WorkSession } from '~/lib/tasksApi'
import { t } from '~/strings'
import './leaf.css'

const TICK_MS = 1000
const SECONDS_PER_MINUTE = 60
const MINUTES_PER_HOUR = 60

/** A running clock reading, e.g. "1:02:09" or "4:30": unlike a rounded duration it visibly ticks. */
export function formatClock(totalSeconds: number): string {
  const total = Math.max(0, Math.floor(totalSeconds))
  const seconds = total % SECONDS_PER_MINUTE
  const minutes = Math.floor(total / SECONDS_PER_MINUTE) % MINUTES_PER_HOUR
  const hours = Math.floor(total / (SECONDS_PER_MINUTE * MINUTES_PER_HOUR))
  const two = (n: number) => String(n).padStart(2, '0')
  return hours > 0 ? `${hours}:${two(minutes)}:${two(seconds)}` : `${minutes}:${two(seconds)}`
}

export function elapsedSeconds(session: WorkSession, nowMs: number): number {
  return (nowMs - parseSqliteUtc(session.started_at).getTime()) / 1000
}

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    setNow(Date.now())
    const id = setInterval(() => setNow(Date.now()), TICK_MS)
    return () => clearInterval(id)
  }, [active])
  return now
}

interface Props {
  sessions: readonly WorkSession[]
  taskById: ReadonlyMap<string, TaskItem>
  onSelect: (taskId: string) => void
}

/** Who is working on what right now, with a clock per lease. Renders nothing when no one is. */
export function WorkingNowPanel({ sessions, taskById, onSelect }: Props) {
  const now = useNow(sessions.length > 0)
  if (sessions.length === 0) return null
  return (
    <section aria-label={t('task_working_now')} className="a-card mb-4">
      <h2 className="a-label">
        {t('task_working_now_count', { n: sessions.length })}
      </h2>
      <ul className="l-stack">
        {sessions.map((s) => {
          const task = taskById.get(s.task_id)
          return (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => onSelect(s.task_id)}
                className="a-drow l-tall l-link w-full"
              >
                <span className="l-work animate-pulse" aria-hidden="true" />
                <span className="l-grow">
                  <span className="block l-clip a-body-sm">{task?.title ?? s.task_id}</span>
                  <span className="block l-clip a-cap a-muted">
                    {s.actor}
                    {s.harness ? ` · ${agentName(s.harness)}` : ''}
                    {s.branch ? `, ${s.branch}` : ''}
                  </span>
                </span>
                <span className="shrink-0 a-body-sm l-num">{formatClock(elapsedSeconds(s, now))}</span>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
