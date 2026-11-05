// Counts only, kept in this browser, never sent (rules 16 to 18).
const KEY = 'hq-noticed'

export const NOTICED_NAMES = [
  'undo.chat_archived',
  'undo.inbox_cleared',
  'undo.limit_deleted',
  'undo.task_archived',
  'undo.task_done',
  'undo.tasks_archived',
  'edit.within_10s',
  'outbox.repeats_dropped',
  'approvals.inbox',
  'approvals.chat',
  'approvals.helpers',
  'approvals.task',
  'data.kb_per_idle_hour',
  'first_load_ms',
] as const
export type NoticedName = (typeof NOTICED_NAMES)[number]

export type Counts = Partial<Record<string, number>>
export interface Noticed {
  version: string
  counts: Counts
  previous?: { version: string; counts: Counts }
}

function load(): Noticed | null {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return null
    const v = JSON.parse(raw) as Noticed
    return v && typeof v.version === 'string' && v.counts && typeof v.counts === 'object' ? v : null
  } catch {
    return null
  }
}

function save(n: Noticed) {
  try {
    localStorage.setItem(KEY, JSON.stringify(n))
  } catch {
    // Storage blocked: the counters are best effort.
  }
}

/** Moves the current numbers beside the new ones when the app version changes. */
function rolled(version: string): Noticed {
  const cur = load()
  if (!cur) return { version, counts: {} }
  if (cur.version === version) return cur
  return { version, counts: {}, previous: { version: cur.version, counts: cur.counts } }
}

export function read(version: string): Noticed {
  const n = rolled(version)
  if (n.version !== load()?.version) save(n)
  return n
}

/** Adds `by` (default 1) to a counter. */
export function count(name: NoticedName, version: string, by = 1) {
  const n = rolled(version)
  n.counts[name] = (n.counts[name] ?? 0) + by
  save(n)
}

/** Sets a measured value such as `first_load_ms` or `data.kb_per_idle_hour`. */
export function record(name: NoticedName, version: string, value: number) {
  const n = rolled(version)
  n.counts[name] = value
  save(n)
}
