import { needsAttention, type AgentStatus, type HarnessSession, type HostWorkspace, type WorkbenchHost } from './sessionsApi'

const AGENT_NAMES: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
  pi: 'Pi',
  opencode: 'OpenCode',
  kimi: 'Kimi',
  qwen: 'Qwen',
  antigravity: 'Antigravity',
  'github-copilot': 'GitHub Copilot',
}
export const DEFAULT_AGENT = 'claude-code'

/** Screen polling periods: fast while the agent is moving or waiting on you, slow once it is ready. */
export const BUSY_POLL_MS = 1_500
export const READY_POLL_MS = 4_000

export const agentName = (harness: string) => AGENT_NAMES[harness] ?? harness

export const isThisComputer = (host: string) => host === 'native' || host === 'local'
export const computerName = (host: string) => (isThisComputer(host) ? 'This computer' : host)

/** The project folder: the last path segment of a working directory, whichever separator it uses. */
export function folderName(cwd: string): string {
  return cwd.split(/[\\/]/).filter(Boolean).pop() ?? ''
}

/** The label if one was set, otherwise "<Agent> in <folder>". */
export function sessionTitle(s: Pick<HarnessSession, 'label' | 'harness' | 'cwd'>): string {
  const label = s.label?.trim()
  if (label) return label
  const folder = folderName(s.cwd)
  return folder ? `${agentName(s.harness)} in ${folder}` : agentName(s.harness)
}

export interface StatusInfo {
  word: string
  /** Set for a running agent whose state is known, so the caller can reuse AGENT_STATUS_CLASS. */
  agent: AgentStatus | null
  /** The session or its computer needs a look: shown in the warning colour. */
  warn: boolean
}

const AGENT_WORDS: Record<AgentStatus, string> = { working: 'Working', blocked: 'Waiting for you', idle: 'Ready', done: 'Finished' }

export function statusInfo(s: Pick<HarnessSession, 'status' | 'agent_status' | 'alive' | 'reachable'>): StatusInfo {
  if (s.reachable === false) return { word: 'Computer offline', agent: null, warn: true }
  if (s.status === 'running') {
    if (s.alive === false) return { word: 'Lost contact', agent: null, warn: true }
    if (!s.agent_status) return { word: 'Starting', agent: null, warn: false }
    return { word: AGENT_WORDS[s.agent_status], agent: s.agent_status, warn: false }
  }
  if (s.status === 'exited') return { word: 'Finished', agent: null, warn: false }
  if (s.status === 'orphaned') return { word: 'Lost contact', agent: null, warn: true }
  return { word: 'Stopped', agent: null, warn: false }
}

const isRunning = (s: Pick<HarnessSession, 'status'>) => s.status === 'running'

/** Running and waiting on the person: blocked at a dialog or with a wake HQ has not handled. */
export const needsYou = (s: HarnessSession) => s.reachable !== false && needsAttention(s)

const activityKey = (s: HarnessSession) => s.last_seen_at ?? s.created_at

const newestFirst = (a: HarnessSession, b: HarnessSession) => activityKey(b).localeCompare(activityKey(a))
const blockedFirst = (a: HarnessSession, b: HarnessSession) =>
  Number(b.agent_status === 'blocked') - Number(a.agent_status === 'blocked') || newestFirst(a, b)
const byFolder = (a: HarnessSession, b: HarnessSession) =>
  folderName(a.cwd).localeCompare(folderName(b.cwd)) || newestFirst(a, b)

export interface WorkbenchGroups {
  needsYou: HarnessSession[]
  working: HarnessSession[]
  past: HarnessSession[]
}

/** Splits the list into the three sections. Archived sessions are dropped unless asked for. */
export function groupSessions(sessions: HarnessSession[], showArchived: boolean): WorkbenchGroups {
  const visible = sessions.filter((s) => showArchived || !s.archived)
  const running = visible.filter(isRunning)
  return {
    needsYou: running.filter(needsYou).sort(blockedFirst),
    working: running.filter((s) => !needsYou(s)).sort(byFolder),
    past: visible.filter((s) => !isRunning(s)).sort(newestFirst),
  }
}

export const needsYouCount = (sessions: HarnessSession[]) => sessions.filter((s) => isRunning(s) && needsYou(s)).length

/** Past sessions that "Archive all" would hide: not running and not archived already. */
export const archivable = (sessions: HarnessSession[]) => sessions.filter((s) => !isRunning(s) && !s.archived)

export const needsYouAria = (count: number) => `${count} ${count === 1 ? 'agent needs' : 'agents need'} you`

/** How often to re-read the terminal: fast while the agent is moving or blocked, slower once it is ready. */
export function screenPollMs(s: Pick<HarnessSession, 'status' | 'agent_status'>): number {
  const busy = s.status === 'running' && (s.agent_status === 'working' || s.agent_status === 'blocked')
  return busy ? BUSY_POLL_MS : READY_POLL_MS
}

/** The last lines of the terminal, for the "waiting for you" callout. */
export function tailLines(text: string, count: number): string {
  return text.trimEnd().split('\n').slice(-count).join('\n')
}

/** Why a computer cannot start an agent, or null when it can. */
export function computerUnavailableReason(h: Pick<WorkbenchHost, 'host' | 'reachable' | 'workspace'>): string | null {
  const name = computerName(h.host)
  if (!h.reachable) return `${name} is offline.`
  if (!h.workspace) return `${name} needs an HQ update before it can run agents here. Update HQ on that computer.`
  return null
}

export interface Crumb {
  label: string
  path: string
}

/** Breadcrumb from the HQ folder down to `path`. Paths outside the HQ folder fall back to just the HQ folder. */
export function crumbs(path: string, root: string): Crumb[] {
  const home: Crumb = { label: 'HQ folder', path: '' }
  if (!path || path === root || !root || !path.startsWith(root)) return [home]
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/'
  const rest = path.slice(root.length).split(/[\\/]/).filter(Boolean)
  let acc = root.replace(/[\\/]+$/, '')
  return [home, ...rest.map((label) => ({ label, path: (acc = `${acc}${sep}${label}`) }))]
}

/** The Windows Explorer line for a folder, shown only for a computer running under WSL. */
export function explorerLine(ws: HostWorkspace | undefined, path: string): string | null {
  if (!ws?.wsl) return null
  const inside = path.startsWith(ws.root) ? path.slice(ws.root.length).split('/').filter(Boolean) : []
  return `Open in Windows Explorer: ${[ws.explorer_path.replace(/\\+$/, ''), ...inside].join('\\')}`
}

export type BlockedKind = 'approval' | 'trust' | 'other'

const CLASSIFY_LINES = 15
const TRUST_PATTERN = /trust|no,\s*exit|yes,\s*proceed[\s\S]*exit/i
const APPROVAL_PATTERN = /do you want to|would you like to|\ballow\b|\(y\/n\)|\[y\/n\]|\bproceed\b|\byes\b[\s\S]*\bno\b/i

/** What a blocked agent is asking. Trust is checked first: its "Yes, proceed" also looks like an approval, but its default is often "No, exit". */
export function classifyBlocked(screenText: string): BlockedKind {
  const tail = tailLines(screenText, CLASSIFY_LINES)
  if (TRUST_PATTERN.test(tail)) return 'trust'
  return APPROVAL_PATTERN.test(tail) ? 'approval' : 'other'
}

/** Holds Approve and Decline after an answer until the screen changes, so a slow refresh cannot send it twice. */
export const answerHeld = (answeredTail: string | null, currentTail: string) => answeredTail !== null && answeredTail === currentTail

export const ANSWER_HOLD_MS = 2_000

/** A Windows extended-length prefix means nothing to the person reading the path. */
export const plainPath = (path: string) => path.replace(/^\\\\\?\\/, '')
