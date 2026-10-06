import { create } from 'zustand'
import { createJSONStorage, persist } from 'zustand/middleware'
import { idbStateStorage } from '~/lib/offlineCache'
import type { ChatAttachment } from '~/lib/chatUploads'
import type { DriverMeta } from '~/lib/sessionsApi'
import { firstUnclaimedStep, type StepCredit } from '~/lib/stepCredits'

// What survives on the device for instant chat loads: recent threads and their latest messages.
const SAVED_THREADS = 20
const SAVED_MESSAGES_PER_THREAD = 50

export interface ToolStep {
  toolCallId: string
  toolName: string
  status: 'running' | 'done'
  startedAt: number
  durationMs?: number
  progressMessages: string[]
  inputArgs?: Record<string, unknown>
  resultOutput?: string
}

export interface Thread {
  threadId: string
  title: string
  status: 'active' | 'archived'
  createdAt: number
  updatedAt: number
  lastMessagePreview?: string
  unreadCount: number
}

export interface ThreadMessage {
  messageId: string
  threadId: string
  role: 'user' | 'assistant'
  content: string
  createdAt: number
  toolSteps?: ToolStep[]
  stepCredits?: StepCredit[]
  reasoning?: string
  attachments?: ChatAttachment[]
  /** The user stopped this reply part way; what streamed before the stop was kept. */
  stopped?: boolean
  /** Posted by HQ for a watched coding-agent session, not in answer to the user. */
  driver?: DriverMeta
  /** Asked by an outside MCP client (hq_ask) rather than typed here: the name it gave itself. */
  viaMcp?: string
}

/** A reply still streaming in one chat. Every chat has its own, so several run at once. */
interface LiveTurn {
  content: string
  reasoning: string
  toolSteps: ToolStep[]
  stepCredits: StepCredit[]
  /** The socket fell behind and skipped events, so this text has holes until the saved reply replaces it. */
  gap?: boolean
}

// A long tool run reports progress for as long as it runs; only the newest lines are ever shown.
const MAX_PROGRESS_MESSAGES = 20

const EMPTY_TURN: LiveTurn = { content: '', reasoning: '', toolSteps: [], stepCredits: [] }

interface ThreadState {
  threads: Thread[]
  activeThreadId: string | null
  threadMessages: Record<string, ThreadMessage[]>
  /** In-flight turns keyed by thread id; a key's presence means that chat is running. */
  live: Record<string, LiveTurn>
  /** Chats whose oldest loaded message is not their first, so a page more can be fetched. */
  olderAvailable: Record<string, boolean>

  setThreads: (threads: Thread[]) => void
  setActiveThread: (threadId: string | null) => void
  addThread: (thread: Thread) => void
  updateThread: (threadId: string, updates: Partial<Thread>) => void
  setThreadMessages: (threadId: string, messages: ThreadMessage[]) => void
  appendMessage: (threadId: string, message: ThreadMessage) => void
  /** Put the server's newest page in place, keeping older pages already loaded when they join up. */
  mergeLatest: (threadId: string, latest: ThreadMessage[], pageWasFull: boolean) => void
  prependMessages: (threadId: string, older: ThreadMessage[], more: boolean) => void
  /** Drop a message and everything after it (an edit or a regenerate replaces them). */
  removeFrom: (threadId: string, messageId: string) => void

  startTurn: (threadId: string) => void
  /** Match live turns to the server's running list; returns chats that finished while we weren't listening. */
  syncRunning: (threadIds: string[]) => string[]
  appendContent: (threadId: string, delta: string) => void
  appendReasoning: (threadId: string, delta: string) => void
  /** Flag every running reply as incomplete after the socket skipped events. */
  markGap: () => void
  startToolStep: (threadId: string, toolCallId: string, toolName: string, inputArgs?: Record<string, unknown>) => void
  updateToolStepProgress: (threadId: string, toolCallId: string, message: string) => void
  endToolStep: (threadId: string, toolCallId?: string, resultOutput?: string) => void
  addStepCredit: (threadId: string, turn: number, delta: number | null) => void
  /**
   * Move a finished turn into the thread's messages, under the server's id when
   * it sent one, and count it unread unless someone saw it arrive. Returns the
   * reply text, or null when the turn left nothing to keep.
   */
  finalizeTurn: (threadId: string, opts: { messageId?: string; seen: boolean; stopped?: boolean }) => string | null
  /** The server saved a user message: swap it in for the local copy with `clientId`, or add it (another device sent it). */
  confirmUserMessage: (threadId: string, message: ThreadMessage, clientId?: string) => void
}

function finishSteps(steps: ToolStep[]): ToolStep[] {
  return steps.map((step) =>
    step.status === 'running' ? { ...step, status: 'done' as const, durationMs: Date.now() - step.startedAt } : step
  )
}

/**
 * Sorts threads with latest meaningful activity first, with deterministic tie-breaking:
 * 1. `updatedAt` descending (latest activity)
 * 2. `createdAt` descending (newest thread)
 * 3. `threadId` descending (stable deterministic tie-break matching backend SQL)
 */
function sortThreads(threads: Thread[]): Thread[] {
  return [...threads].sort((a, b) => {
    if (b.updatedAt !== a.updatedAt) {
      return b.updatedAt - a.updatedAt
    }
    if (b.createdAt !== a.createdAt) {
      return b.createdAt - a.createdAt
    }
    return b.threadId.localeCompare(a.threadId)
  })
}

export const useThreadStore = create<ThreadState>()(
  persist(
    (set, get) => {
      const patchLive = (threadId: string, patch: (turn: LiveTurn) => Partial<LiveTurn>) =>
        set((s) => {
          const turn = s.live[threadId] ?? EMPTY_TURN
          return { live: { ...s.live, [threadId]: { ...turn, ...patch(turn) } } }
        })

      return {
        threads: [],
        activeThreadId: null,
        threadMessages: {},
        live: {},
        olderAvailable: {},

        setThreads: (threads) => set({ threads: sortThreads(threads) }),
        setActiveThread: (threadId) =>
          set((s) => ({
            activeThreadId: threadId,
            threads: s.threads.map((t) => (t.threadId === threadId ? { ...t, unreadCount: 0 } : t)),
          })),
        addThread: (thread) =>
          set((s) => ({
            threads: sortThreads([thread, ...s.threads.filter((t) => t.threadId !== thread.threadId)]),
          })),
        updateThread: (threadId, updates) =>
          set((s) => ({
            threads: sortThreads(s.threads.map((t) => (t.threadId === threadId ? { ...t, ...updates } : t))),
          })),
        setThreadMessages: (threadId, messages) =>
          set((s) => ({ threadMessages: { ...s.threadMessages, [threadId]: messages } })),
        mergeLatest: (threadId, latest, pageWasFull) =>
          set((s) => {
            const current = s.threadMessages[threadId] ?? []
            // A short page is the whole history, so nothing loaded before it can still exist.
            const join = pageWasFull && latest.length > 0 ? current.findIndex((m) => m.messageId === latest[0].messageId) : -1
            const kept = join > 0 ? current.slice(0, join) : []
            const olderLoaded = kept.length > 0 ? s.olderAvailable[threadId] ?? false : pageWasFull
            return {
              threadMessages: { ...s.threadMessages, [threadId]: [...kept, ...latest] },
              olderAvailable: { ...s.olderAvailable, [threadId]: olderLoaded },
            }
          }),
        prependMessages: (threadId, older, more) =>
          set((s) => {
            const current = s.threadMessages[threadId] ?? []
            const known = new Set(current.map((m) => m.messageId))
            const fresh = older.filter((m) => !known.has(m.messageId))
            return {
              threadMessages: { ...s.threadMessages, [threadId]: [...fresh, ...current] },
              olderAvailable: { ...s.olderAvailable, [threadId]: more },
            }
          }),
        removeFrom: (threadId, messageId) =>
          set((s) => {
            const current = s.threadMessages[threadId] ?? []
            const at = current.findIndex((m) => m.messageId === messageId)
            if (at < 0) return {}
            return { threadMessages: { ...s.threadMessages, [threadId]: current.slice(0, at) } }
          }),
        appendMessage: (threadId, message) =>
          set((s) => {
            const preview = message.content.length > 80 ? `${message.content.slice(0, 80)}...` : message.content
            const threads = s.threads.map((t) =>
              t.threadId === threadId
                ? {
                    ...t,
                    updatedAt: Math.max(t.updatedAt, message.createdAt || Date.now()),
                    lastMessagePreview: preview || t.lastMessagePreview,
                  }
                : t
            )
            return {
              threads: sortThreads(threads),
              threadMessages: {
                ...s.threadMessages,
                [threadId]: [...(s.threadMessages[threadId] ?? []), message],
              },
            }
          }),

        startTurn: (threadId) => patchLive(threadId, () => ({})),
        syncRunning: (threadIds) => {
          const current = get().live
          const finished = Object.keys(current).filter((id) => !threadIds.includes(id))
          const live: Record<string, LiveTurn> = {}
          for (const id of threadIds) live[id] = current[id] ?? EMPTY_TURN
          set({ live })
          return finished
        },
        appendContent: (threadId, delta) => patchLive(threadId, (t) => ({ content: t.content + delta })),
        appendReasoning: (threadId, delta) => patchLive(threadId, (t) => ({ reasoning: t.reasoning + delta })),
        markGap: () => set((s) => ({ live: Object.fromEntries(Object.entries(s.live).map(([id, t]) => [id, { ...t, gap: true }])) })),
        startToolStep: (threadId, toolCallId, toolName, inputArgs) =>
          patchLive(threadId, (t) => ({
            // A repeated id (a resend after a reconnect) must not add a second row with the same key.
            toolSteps: t.toolSteps.some((step) => step.toolCallId === toolCallId) ? t.toolSteps : [...t.toolSteps, {
              toolCallId, toolName, status: 'running', startedAt: Date.now(), progressMessages: [], inputArgs,
            }],
          })),
        updateToolStepProgress: (threadId, toolCallId, message) =>
          patchLive(threadId, (t) => ({
            toolSteps: t.toolSteps.map((step) =>
              step.toolCallId === toolCallId
                ? { ...step, progressMessages: [...step.progressMessages, message].slice(-MAX_PROGRESS_MESSAGES) }
                : step
            ),
          })),
        endToolStep: (threadId, toolCallId, resultOutput) =>
          patchLive(threadId, (t) => ({
            toolSteps: toolCallId
              ? t.toolSteps.map((step) =>
                  step.toolCallId === toolCallId
                    ? { ...step, status: 'done' as const, durationMs: Date.now() - step.startedAt, resultOutput: resultOutput ?? step.resultOutput }
                    : step
                )
              : finishSteps(t.toolSteps),
          })),
        addStepCredit: (threadId, turn, delta) =>
          patchLive(threadId, (t) => ({
            stepCredits: [
              ...t.stepCredits,
              {
                turn,
                delta,
                toolCallId: firstUnclaimedStep(t.toolSteps.map((s) => s.toolCallId), t.stepCredits),
                stepsSeen: t.toolSteps.length,
              },
            ],
          })),
        finalizeTurn: (threadId, { messageId, seen, stopped }) => {
          const turn = get().live[threadId]
          set((s) => {
            const { [threadId]: _done, ...live } = s.live
            return { live }
          })
          if (!turn || (!turn.content.trim() && !turn.reasoning.trim() && turn.toolSteps.length === 0)) return null
          get().appendMessage(threadId, {
            messageId: messageId ?? `streamed-${Date.now()}`,
            threadId,
            role: 'assistant',
            content: turn.content,
            createdAt: Date.now(),
            toolSteps: turn.toolSteps.length > 0 ? finishSteps(turn.toolSteps) : undefined,
            stepCredits: turn.stepCredits.length > 0 ? turn.stepCredits : undefined,
            reasoning: turn.reasoning.trim() ? turn.reasoning : undefined,
            stopped: stopped || undefined,
          })
          const preview = turn.content.length > 80 ? `${turn.content.slice(0, 80)}...` : turn.content
          set((s) => ({
            threads: sortThreads(
              s.threads.map((t) =>
                t.threadId === threadId
                  ? {
                      ...t,
                      updatedAt: Date.now(),
                      lastMessagePreview: preview || t.lastMessagePreview,
                      unreadCount: seen && s.activeThreadId === threadId ? 0 : t.unreadCount + 1,
                    }
                  : t
              )
            ),
          }))
          return turn.content
        },
        confirmUserMessage: (threadId, message, clientId) =>
          set((s) => {
            const list = s.threadMessages[threadId] ?? []
            if (list.some((m) => m.messageId === message.messageId)) return {}
            const local = clientId ? list.findIndex((m) => m.messageId === clientId) : -1
            const next = local >= 0 ? list.map((m, i) => (i === local ? { ...m, messageId: message.messageId } : m)) : [...list, message]
            const preview = message.content.length > 80 ? `${message.content.slice(0, 80)}...` : message.content
            const threads = s.threads.map((t) =>
              t.threadId === threadId
                ? {
                    ...t,
                    updatedAt: Math.max(t.updatedAt, message.createdAt || Date.now()),
                    lastMessagePreview: preview || t.lastMessagePreview,
                  }
                : t
            )
            return {
              threads: sortThreads(threads),
              threadMessages: { ...s.threadMessages, [threadId]: next },
            }
          }),
      }
    },
    {
      name: 'hq-thread-store',
      storage: createJSONStorage(() => idbStateStorage),
      partialize: (s) => {
        const sorted = sortThreads(s.threads)
        const recent = sorted.slice(0, SAVED_THREADS)
        const threadMessages: Record<string, ThreadMessage[]> = {}
        for (const t of recent) {
          const msgs = s.threadMessages[t.threadId]
          if (msgs?.length) threadMessages[t.threadId] = msgs.slice(-SAVED_MESSAGES_PER_THREAD)
        }
        return { threads: sorted, activeThreadId: s.activeThreadId, threadMessages }
      },
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<ThreadState>
        // The server's data can land before the async cache read; the cache must never replace it.
        const threads = sortThreads(current.threads.length ? current.threads : (p.threads ?? []))
        const threadMessages = { ...(p.threadMessages ?? {}), ...current.threadMessages }
        const activeThreadId = current.activeThreadId ?? p.activeThreadId ?? null
        return { ...current, ...p, threads, threadMessages, activeThreadId }
      },
    }
  )
)

/** The in-flight turn for one chat, or undefined when it is idle. */
export const useLiveTurn = (threadId: string | null | undefined) =>
  useThreadStore((s) => (threadId ? s.live[threadId] : undefined))
