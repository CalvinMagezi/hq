import { parseSqliteUtc, type WorkSession } from '~/lib/tasksApi'

const SECONDS_PER_MINUTE = 60
const SECONDS_PER_HOUR = 3600
const SECONDS_PER_DAY = 86_400
const MS_PER_DAY = 86_400_000

/** "45s", "12m", "2h 5m", "3d 4h": the two largest units, never more. */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || Number.isNaN(seconds)) return 'unknown'
  const total = Math.max(0, Math.round(seconds))
  if (total < SECONDS_PER_MINUTE) return `${total}s`
  if (total < SECONDS_PER_HOUR) return `${Math.floor(total / SECONDS_PER_MINUTE)}m`
  if (total < SECONDS_PER_DAY) {
    const hours = Math.floor(total / SECONDS_PER_HOUR)
    const minutes = Math.floor((total % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE)
    return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`
  }
  const days = Math.floor(total / SECONDS_PER_DAY)
  const hours = Math.floor((total % SECONDS_PER_DAY) / SECONDS_PER_HOUR)
  return hours === 0 ? `${days}d` : `${days}d ${hours}h`
}

/** Minutes as a duration, with the sign kept for variances ("+1h", "-30m"). */
export function formatSignedMinutes(minutes: number): string {
  const text = formatDuration(Math.abs(minutes) * SECONDS_PER_MINUTE)
  return minutes > 0 ? `+${text}` : minutes < 0 ? `-${text}` : text
}

/** A stored UTC timestamp in the viewer's timezone, with the offset so it is never ambiguous. */
export function formatLocalTime(stored: string): string {
  return parseSqliteUtc(stored).toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZoneName: 'short',
  })
}

/** Inclusive UTC day numbers a lease covers, from its start to its last sign of life. */
export interface ActualSpan {
  start: number
  end: number
  live: boolean
}

/** Work leases as day spans per task, so the timeline can draw what happened beside what was planned. */
export function actualSpans(sessions: WorkSession[]): Map<string, ActualSpan[]> {
  const out = new Map<string, ActualSpan[]>()
  for (const session of sessions) {
    const startMs = parseSqliteUtc(session.started_at).getTime()
    if (Number.isNaN(startMs)) continue
    const endMs = startMs + Math.max(0, session.active_seconds) * 1000
    const span: ActualSpan = {
      start: Math.floor(startMs / MS_PER_DAY),
      end: Math.floor(endMs / MS_PER_DAY),
      live: session.ended_at === null,
    }
    out.set(session.task_id, [...(out.get(session.task_id) ?? []), span])
  }
  return out
}
