import { useEffect, useState } from 'react'
import { parseSqliteUtc, type TaskItem, type WorkSession } from '~/lib/tasksApi'

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
    <section aria-label="Working now" className="mb-4 rounded-2xl hq-card p-2">
      <h2 className="px-2 pt-1 pb-1.5 text-[11px] font-bold uppercase tracking-wider text-neutral-400">
        Working now ({sessions.length})
      </h2>
      <ul className="space-y-1">
        {sessions.map((s) => {
          const task = taskById.get(s.task_id)
          return (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => onSelect(s.task_id)}
                className="w-full min-w-0 flex items-center gap-2 rounded-xl px-2 py-2 text-left hover:bg-white/5"
              >
                <span className="w-1.5 h-1.5 rounded-full bg-current text-neutral-200 animate-pulse shrink-0" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs text-neutral-100">{task?.title ?? s.task_id}</span>
                  <span className="block truncate text-[11px] text-neutral-500">
                    {s.actor}
                    {s.harness ? ` on ${s.harness}` : ''}
                    {s.branch ? `, ${s.branch}` : ''}
                  </span>
                </span>
                <span className="shrink-0 text-xs tabular-nums text-neutral-200">{formatClock(elapsedSeconds(s, now))}</span>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
