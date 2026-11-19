import { hqJson } from './hqAuth'

export type HelperRunState =
  | 'queued'
  | 'running'
  | 'stalled'
  | 'done'
  | 'partial'
  | 'blocked'
  | 'failed'
  | 'timed_out'
  | 'cancelled'
  | 'interrupted'
  | 'rejected'

export interface HelperRun {
  id: string
  thread_id: string | null
  task: string
  role: string
  goal: string
  state: HelperRunState
  exec_status?: string
  accept_status?: string
  detached: boolean
  started_at: number | null
  last_activity_at?: number | null
  settled_at: number | null
  result_link: string | null
  result_preview: string | null
  error: string | null
  blocker: string | null
  missing_deliverables?: string[]
}

export function isHelperRun(v: unknown): v is HelperRun {
  if (!v || typeof v !== 'object') return false
  const o = v as Record<string, unknown>
  return typeof o.id === 'string' && typeof o.state === 'string'
}

export async function fetchThreadHelperRuns(threadId: string): Promise<HelperRun[]> {
  const res = await hqJson<{ runs?: HelperRun[] }>(`/api/threads/${encodeURIComponent(threadId)}/helper-runs`)
  return Array.isArray(res.runs) ? res.runs : []
}

export async function searchHelperRuns(q: string, limit = 20): Promise<{ runs: HelperRun[]; count: number }> {
  const res = await hqJson<{ runs?: HelperRun[]; count?: number }>(
    `/api/helper-runs?q=${encodeURIComponent(q)}&limit=${limit}`,
  )
  const runs = Array.isArray(res.runs) ? res.runs : []
  return { runs, count: typeof res.count === 'number' ? res.count : runs.length }
}
