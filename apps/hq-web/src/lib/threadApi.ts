import { hqJson } from './hqAuth'
import { useThreadStore, type Thread, type ThreadMessage, type ToolStep } from '~/store/threadStore'
import { splitAttachments } from './chatUploads'
import type { DriverMeta } from './sessionsApi'
import { parseStepCredits } from './stepCredits'

/** A list preview without the attachment marker; a files-only message reads as its file count. */
function cleanPreview(preview: string | undefined): string | undefined {
  if (!preview) return preview
  const { text, attachments } = splitAttachments(preview)
  if (text.trim() || attachments.length === 0) return text.trim()
  return `[${attachments.length} file${attachments.length === 1 ? '' : 's'}]`
}

interface ApiThread {
  thread_id: string
  title: string
  status: string
  created_at: string
  updated_at: string
  last_message_preview?: string
  unread_count: number
}

function toThread(t: ApiThread): Thread {
  return {
    threadId: t.thread_id,
    title: t.title || 'New chat',
    status: (t.status as 'active' | 'archived') || 'active',
    createdAt: new Date(t.created_at).getTime(),
    updatedAt: new Date(t.updated_at).getTime(),
    lastMessagePreview: cleanPreview(t.last_message_preview),
    unreadCount: 0,
  }
}

/** One page of history; a full page means older messages may exist. */
export const MESSAGE_PAGE_SIZE = 50

/** A tool call as the server saved it with its reply (arguments and result are capped and redacted). */
interface ApiToolStep {
  id: string
  name: string
  args?: string
  result?: string
  duration_ms?: number
}

/** Set on replies the backend posted for a watched coding-agent session. */
interface ApiDriverMeta {
  session_id?: string
  reason?: string
  mode?: string
}

/** Set on a question an MCP client put to HQ with hq_ask. */
interface ApiSourceMeta {
  kind?: string
  caller?: string
}

interface ApiMessageMeta {
  reasoning?: string
  tool_steps?: ApiToolStep[]
  step_credits?: unknown
  stopped?: boolean
  driver?: ApiDriverMeta | null
  source?: ApiSourceMeta | null
}

export interface ApiMessage {
  message_id: string
  thread_id: string
  role: string
  content: string
  created_at: string
  meta?: ApiMessageMeta | null
}

/** Tool arguments arrive as JSON text; anything unparsable is shown as-is. */
export function parseToolArgs(raw: string | undefined): Record<string, unknown> | undefined {
  if (!raw) return undefined
  try {
    const parsed = JSON.parse(raw) as unknown
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : { value: parsed }
  } catch {
    return { raw }
  }
}

function toToolStep(step: ApiToolStep): ToolStep {
  return {
    toolCallId: step.id,
    toolName: step.name,
    status: 'done',
    startedAt: 0,
    durationMs: step.duration_ms,
    progressMessages: [],
    inputArgs: parseToolArgs(step.args),
    resultOutput: step.result,
  }
}

function toViaMcp(s: ApiSourceMeta | null | undefined): string | undefined {
  return s?.kind === 'mcp' ? s.caller?.trim() || 'mcp' : undefined
}

function toDriver(d: ApiDriverMeta | null | undefined): DriverMeta | undefined {
  if (!d?.session_id) return undefined
  return { sessionId: d.session_id, reason: d.reason ?? '', mode: d.mode === 'drive' ? 'drive' : 'update' }
}

/** A saved message in the store's shape: attachments split out, tool steps and reasoning from its meta. */
export function toThreadMessage(m: ApiMessage): ThreadMessage {
  const { text, attachments } = splitAttachments(m.content)
  const steps = m.meta?.tool_steps ?? []
  return {
    messageId: m.message_id,
    threadId: m.thread_id,
    role: m.role as 'user' | 'assistant',
    content: text,
    createdAt: new Date(m.created_at).getTime(),
    attachments: attachments.length > 0 ? attachments : undefined,
    toolSteps: steps.length > 0 ? steps.map(toToolStep) : undefined,
    stepCredits: parseStepCredits(m.meta?.step_credits),
    reasoning: m.meta?.reasoning || undefined,
    stopped: m.meta?.stopped || undefined,
    driver: toDriver(m.meta?.driver),
    viaMcp: toViaMcp(m.meta?.source),
  }
}

export const threadApi = {
  /** Web chats only; `running` lists chats with a reply still in flight. */
  listThreads: (): Promise<{ threads: ApiThread[]; running?: string[] }> => hqJson('/api/threads'),

  createThread: (title: string): Promise<ApiThread> => hqJson('/api/threads', 'POST', { title }),

  /** The newest `limit` messages, or the `limit` before message `before`; oldest first either way. */
  getMessages: (threadId: string, limit = MESSAGE_PAGE_SIZE, before?: string): Promise<ApiMessage[]> => {
    const cursor = before ? `&before=${encodeURIComponent(before)}` : ''
    return hqJson(`/api/threads/${encodeURIComponent(threadId)}/messages?limit=${limit}${cursor}`)
  },

  archiveThread: (threadId: string): Promise<void> =>
    hqJson(`/api/threads/${encodeURIComponent(threadId)}/archive`, 'POST'),
}

/** Reload the chat list, and mark chats whose reply is still running on the server. */
export async function refreshThreads(): Promise<Thread[]> {
  const res = await threadApi.listThreads()
  const threads = (res.threads ?? []).map(toThread)
  const store = useThreadStore.getState()
  store.setThreads(threads)
  const finished = store.syncRunning(res.running ?? [])
  await Promise.all(finished.map((id) => loadMessages(id).catch(() => {})))
  return threads
}

/**
 * Load a chat's newest saved messages, unless a reply is streaming into it
 * right now. Older pages already loaded are kept.
 */
export async function loadMessages(threadId: string): Promise<void> {
  const remote = await threadApi.getMessages(threadId, MESSAGE_PAGE_SIZE)
  const store = useThreadStore.getState()
  if (store.live[threadId]) return
  store.mergeLatest(threadId, remote.map(toThreadMessage), remote.length === MESSAGE_PAGE_SIZE)
}

/** Fetch the page before the oldest loaded message. Resolves to whether more are left. */
export async function loadOlderMessages(threadId: string): Promise<boolean> {
  const oldest = useThreadStore.getState().threadMessages[threadId]?.find((m) => !m.messageId.startsWith('local-'))
  if (!oldest) return false
  const older = await threadApi.getMessages(threadId, MESSAGE_PAGE_SIZE, oldest.messageId)
  const more = older.length === MESSAGE_PAGE_SIZE
  useThreadStore.getState().prependMessages(threadId, older.map(toThreadMessage), more)
  return more
}

/** Create an empty chat and put it at the top of the list. */
export async function newThread(title = 'New chat'): Promise<string> {
  const t = await threadApi.createThread(title)
  const store = useThreadStore.getState()
  store.addThread(toThread(t))
  store.setThreadMessages(t.thread_id, [])
  return t.thread_id
}
