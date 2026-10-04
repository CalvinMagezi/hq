import { useHQStore } from '~/store/hqStore'
import { useThreadStore } from '~/store/threadStore'
import { loadMessages, parseToolArgs, refreshThreads, toThreadMessage, type ApiMessage } from './threadApi'
import { alertReplyDone, chatOnScreen } from './replyAlerts'
import { restoreDraft } from '~/components/chat/draftStore'

type ChatEvent = Record<string, unknown>

const CHAT_EVENTS = new Set([
  'turn_start',
  'text_delta',
  'reasoning_delta',
  'error',
  'turn_end',
  'thread_title',
  'tool_start',
  'tool_progress',
  'tool_end',
  'step_credits',
  'chat_rejected',
  'relay_message',
  'stream_lag',
])

const str = (v: unknown) => (typeof v === 'string' ? v : undefined)

// Tokens arrive far faster than a screen needs them; each store write re-renders the chat and re-parses the reply.
const DELTA_FLUSH_MS = 60
const pending = new Map<string, { content: string; reasoning: string }>()
let flushTimer: ReturnType<typeof setTimeout> | null = null
let fallbackIds = 0

/** Writes every buffered delta to the store. Any other event calls this first so order is kept. */
export function flushPendingDeltas() {
  if (flushTimer) clearTimeout(flushTimer)
  flushTimer = null
  const store = useThreadStore.getState()
  for (const [tid, buffered] of pending) {
    if (buffered.content) store.appendContent(tid, buffered.content)
    if (buffered.reasoning) store.appendReasoning(tid, buffered.reasoning)
  }
  pending.clear()
}

function queueDelta(tid: string, field: 'content' | 'reasoning', delta: string) {
  const buffered = pending.get(tid) ?? { content: '', reasoning: '' }
  buffered[field] += delta
  pending.set(tid, buffered)
  flushTimer ??= setTimeout(flushPendingDeltas, DELTA_FLUSH_MS)
}

/**
 * Applies one chat event from the socket to the thread store. Returns false
 * for anything that is not a chat event, so the caller can handle it.
 * Events carry their chat's id, so every chat streams on its own.
 */
export function handleChatEvent(msg: ChatEvent): boolean {
  const type = str(msg.type)
  if (!type || !CHAT_EVENTS.has(type)) return false
  if (type === 'stream_lag') {
    onStreamLag()
    return true
  }
  const store = useThreadStore.getState()
  const tid = str(msg.thread_id) ?? store.activeThreadId
  if (!tid) return true
  if (type === 'text_delta' || type === 'reasoning_delta') {
    queueDelta(tid, type === 'text_delta' ? 'content' : 'reasoning', str(msg.content) ?? '')
    return true
  }
  flushPendingDeltas()
  try {
    applyEvent(type, msg, tid)
  } catch (err) {
    console.error(`chatEvents: could not apply a ${type} event`, err)
    if (type === 'turn_end') dropLiveTurn(tid)
    useHQStore.getState().setSystemNotice('A chat update could not be read. Reload the chat if it looks wrong.')
  }
  return true
}

/** The server says this socket skipped events: mark replies as incomplete and reload from the saved copies. */
function onStreamLag() {
  flushPendingDeltas()
  useThreadStore.getState().markGap()
  void refreshThreads().catch((err) => console.warn('chatEvents: could not resync after lag', err))
}

/** A turn that cannot be finalized must still stop looking busy; the saved copy is reloaded. */
function dropLiveTurn(tid: string) {
  useThreadStore.setState((s) => {
    const { [tid]: _dropped, ...live } = s.live
    return { live }
  })
  void loadMessages(tid).catch(() => {})
}

function applyEvent(type: string, msg: ChatEvent, tid: string) {
  const store = useThreadStore.getState()

  // A chat started on another device is new to this one until the list reloads.
  const unknownThread = !store.threads.some((t) => t.threadId === tid)
  if ((type === 'turn_start' || type === 'thread_title' || type === 'relay_message') && unknownThread) void refreshThreads().catch(() => {})

  switch (type) {
    case 'turn_start': {
      const saved = msg.user_message as ApiMessage | undefined
      const replaced = str(msg.replace_from)
      // An edit or regenerate from any tab replaced these; the sender already dropped them.
      if (replaced) store.removeFrom(tid, replaced)
      try {
        if (saved) store.confirmUserMessage(tid, toThreadMessage(saved), str(msg.client_id))
      } finally {
        // A saved message that cannot be read must not keep the reply from showing as running.
        store.startTurn(tid)
      }
      break
    }
    case 'error':
      store.appendContent(tid, `\n\n**Error:** ${str(msg.content) ?? 'unknown'}\n\n`)
      break
    case 'turn_end':
      finishTurn(tid, str(msg.message_id), msg.stopped === true)
      break
    case 'thread_title':
      store.updateThread(tid, { title: str(msg.title) ?? '' })
      break
    case 'tool_start':
      store.startToolStep(
        tid,
        str(msg.tool_call_id) ?? `call-${Date.now()}-${fallbackIds++}`,
        str(msg.tool_name) ?? 'tool',
        parseToolArgs(str(msg.args)),
      )
      break
    case 'tool_progress': {
      const callId = str(msg.tool_call_id)
      if (callId) store.updateToolStepProgress(tid, callId, str(msg.message) ?? '')
      break
    }
    case 'tool_end':
      store.endToolStep(tid, str(msg.tool_call_id), str(msg.result))
      break
    case 'step_credits':
      store.addStepCredit(tid, typeof msg.turn === 'number' ? msg.turn : 0, typeof msg.delta === 'number' ? msg.delta : null)
      break
    case 'chat_rejected':
      rejectSend(tid, str(msg.client_id), str(msg.reason) ?? 'The message was not sent', msg.running === true)
      break
    case 'relay_message': {
      const role = str(msg.role) ?? 'assistant'
      const content = str(msg.content) ?? ''
      const createdAt = typeof msg.created_at === 'string' ? new Date(msg.created_at).getTime() : Date.now()
      store.appendMessage(tid, {
        messageId: str(msg.message_id) ?? `relay-${Date.now()}`,
        threadId: tid,
        role: role as 'user' | 'assistant',
        content,
        createdAt,
      })
      break
    }
  }
}

/**
 * The server refused a send (say, another tab started a reply first). Every
 * tab hears it; only the one holding the local copy acts. That copy goes and
 * its text is back in the composer if the composer is still empty.
 */
export function rejectSend(tid: string, clientId: string | undefined, reason: string, running: boolean) {
  const store = useThreadStore.getState()
  const local = clientId ? store.threadMessages[tid]?.find((m) => m.messageId === clientId) : undefined
  if (!local || !clientId) return
  store.removeFrom(tid, clientId)
  // The optimistic turn is empty; ending it saves nothing. A reply that really is running keeps streaming.
  if (!running) store.finalizeTurn(tid, { seen: true })
  restoreDraft(tid, local.content)
  // An edit already dropped the messages it would replace; the server still has them.
  void loadMessages(tid).catch(() => {})
  useHQStore.getState().setSystemNotice(reason)
}

/** Saves the reply into the chat, and counts and announces it when nobody is looking. */
function finishTurn(tid: string, messageId: string | undefined, stopped: boolean) {
  const hq = useHQStore.getState()
  const seen = chatOnScreen(hq.globalChatOpen)
  const reply = useThreadStore.getState().finalizeTurn(tid, { messageId, seen, stopped })
  // A socket that dropped mid-reply (a phone in the background) missed some
  // deltas; the server's saved copy is whole, so it replaces the local one.
  void loadMessages(tid).catch(() => {})
  if (seen || reply === null || stopped) return
  hq.bumpChatUnread()
  const title = useThreadStore.getState().threads.find((t) => t.threadId === tid)?.title ?? 'HQ'
  void alertReplyDone(tid, title, reply)
}
