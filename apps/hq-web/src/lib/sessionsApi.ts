import { hqJson } from './hqAuth'
import type { TaskStatus } from './tasksApi'

export type SessionStatus = 'running' | 'exited' | 'stopped' | 'orphaned'
export type AgentStatus = 'idle' | 'working' | 'blocked' | 'done'

/** A Herdr coding-agent session a chat is watching. Timestamps are SQLite UTC strings. */
export interface WatchedSession {
  id: string
  harness: string
  label: string
  host: string
  cwd: string
  status: SessionStatus
  agent_status: AgentStatus | null
  last_seen_at: string | null
  /** HQ answers the agent and approves its prompts toward the task's goal. */
  drive: boolean
  /** Whether HQ is driving the session or only watching it. */
  mode: 'drive' | 'observe'
  /** What the session is for, and what would show it is done. Drive needs both to be specific. */
  goal: string | null
  done_criteria: string | null
  /** Why Drive cannot be on yet: empty when the goal and definition of done pass the gate. */
  drive_blocked_by: string[]
  /** Why a limit or the gate last switched Drive off; null while Drive is on or the user switched it off. */
  drive_off_reason: string | null
  /** Instructions HQ has sent since Drive was last switched on by the user. */
  nudges_sent: number
  pending_wake: string | null
  last_driven_at: string | null
  created_at: string
  task: { id: string; display_id: string; title: string; status: TaskStatus } | null
}

/** Why the backend posted a message into a chat on a session's behalf. */
export interface DriverMeta {
  sessionId: string
  reason: string
  mode: 'update' | 'drive'
}

/** A watched session waiting on someone: blocked at a prompt, or with a wake HQ has not handled yet. */
export const needsAttention = (s: WatchedSession) =>
  s.status === 'running' && (s.agent_status === 'blocked' || s.pending_wake !== null)

/** The Watching button's accessible name, since on a phone it shows only an icon and numbers. */
export function watchingLabel(sessions: WatchedSession[]): string {
  const noun = sessions.length === 1 ? 'session' : 'sessions'
  const waiting = sessions.filter(needsAttention).length
  return waiting > 0 ? `Watching ${sessions.length} ${noun}, ${waiting} need attention` : `Watching ${sessions.length} ${noun}`
}

const sessionPath = (id: string) => `/api/harness-sessions/${encodeURIComponent(id)}`

export const sessionsApi = {
  listForThread: async (threadId: string): Promise<WatchedSession[]> => {
    const res = await hqJson<{ sessions?: WatchedSession[] }>(`/api/threads/${encodeURIComponent(threadId)}/sessions`)
    return res.sessions ?? []
  },

  setDrive: async (id: string, drive: boolean): Promise<WatchedSession> => {
    const res = await hqJson<{ session: WatchedSession }>(`${sessionPath(id)}/drive`, 'POST', { drive })
    return res.session
  },

  unwatch: (id: string): Promise<{ ok: boolean }> => hqJson(`${sessionPath(id)}/unwatch`, 'POST'),
}

/** A registry row with its live state, as the global Sessions page lists it. */
export interface HarnessSession extends WatchedSession {
  agent_name: string
  /** The chat watching it; null means nobody does, so it can be adopted. */
  owner_thread: string | null
  /** Whether a running session's agent exists right now; null when its host could not be asked. */
  alive?: boolean | null
  /** False when the host is unreachable (the agent state is then unknown, not exited). */
  reachable?: boolean
  detail?: string
}

export interface SessionFilters {
  task_id?: string
  status?: SessionStatus
  host?: string
}

export interface ScreenText {
  session_id: string
  /** `live` while the agent runs, `snapshot` for the last text stored after it ended. */
  source: 'live' | 'snapshot'
  lines: string[]
}

export type SendPayload = { text: string; keys?: undefined } | { keys: string[]; text?: undefined }

export interface AdoptResult {
  session: HarnessSession
  thread_id: string
  created_thread: boolean
  already_watched: boolean
}

/** Keys the send box offers for answering a dialog; Herdr's logical key names. */
export const QUICK_KEYS = ['enter', 'esc', 'up', 'down', 'y', 'n'] as const

/** Kills what the agent is doing, so the send box asks for a second tap before sending it. */
export const INTERRUPT_KEY = 'ctrl+c'

/** The server accepts only logical key names like these (see herdr::validate_keys). */
export const KEY_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9+_-]{0,31}$/

const queryString = (filters: SessionFilters) => {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(filters)) if (v) q.set(k, v)
  const s = q.toString()
  return s ? `?${s}` : ''
}

/** The command that opens the session's Herdr on its machine. A remote host's name is used as the ssh target. */
export function attachCommand(s: Pick<HarnessSession, 'host'>): string {
  return s.host === 'local' ? 'herdr' : `ssh ${s.host} -t herdr`
}

/** Caveat shown beside the attach command: HQ names hosts, ssh knows addresses, so the name must resolve as an ssh alias. */
export function attachCaveat(s: Pick<HarnessSession, 'host'>): string | null {
  return s.host === 'local' ? null : `assumes "${s.host}" is an ssh alias on this machine`
}

/** The session is blocked at a dialog, so text is refused and keys must answer it. */
export const isBlocked = (s: Pick<HarnessSession, 'status' | 'agent_status'>) =>
  s.status === 'running' && s.agent_status === 'blocked'

/** Whether anything can be typed into the session right now. */
export const canSend = (s: Pick<HarnessSession, 'status' | 'alive' | 'reachable'>) =>
  s.status === 'running' && s.alive !== false && s.reachable !== false

export const globalSessionsApi = {
  list: async (filters: SessionFilters = {}): Promise<HarnessSession[]> => {
    const res = await hqJson<{ sessions?: HarnessSession[] }>(`/api/harness-sessions${queryString(filters)}`)
    return res.sessions ?? []
  },

  get: async (id: string): Promise<HarnessSession> => (await hqJson<{ session: HarnessSession }>(sessionPath(id))).session,

  screen: (id: string, lines: number): Promise<ScreenText> => hqJson(`${sessionPath(id)}/screen?lines=${lines}`),

  send: (id: string, payload: SendPayload): Promise<{ sent?: string; keys?: string[]; note?: string }> =>
    hqJson(`${sessionPath(id)}/send`, 'POST', payload),

  adopt: (id: string, threadId?: string): Promise<AdoptResult> =>
    hqJson(`${sessionPath(id)}/adopt`, 'POST', threadId ? { thread_id: threadId } : {}),
}
