import { useCallback, useEffect, useRef, useState } from 'react'
import { newClientId } from '~/lib/outbox'
import { Paperclip, PencilLine, X } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { useThreadStore, useLiveTurn, type ThreadMessage } from '~/store/threadStore'
import { useWS } from '~/context/WebSocketContext'
import { toastUndo, toastAlert, toastNotice } from '~/lib/toast'
import { failureLine } from '~/lib/version'
import { t } from '~/strings'
import { useSharedConnection } from '~/lib/connection'
import { isSharedComputer } from '~/lib/sharedComputer'
import { bannerKey, sendFailureKey, stopFailureKey, stopReply } from '~/lib/chatConnection'
import { threadApi, refreshThreads, loadMessages, newThread } from '~/lib/threadApi'
import { rejectSend } from '~/lib/chatEvents'
import { MAX_ATTACHMENTS, type ChatAttachment } from '~/lib/chatUploads'
import { parseSlashCommand, isSlashInput } from './SlashCommandPalette'
import { ThreadSidebar } from './ThreadSidebar'
import { ChatComposer } from './ChatComposer'
import { ChatHeader } from './ChatHeader'
import { MessageList, isSaved } from './MessageList'
import { OutboxGroup } from './OutboxGroup'
import { useChatOutbox } from './useChatOutbox'
import { useDraft } from './draftStore'
import { useFileDrop } from './useFileDrop'
import { useThreadSessions } from './useThreadSessions'
import { SessionsPanel } from './SessionsPanel'

const FOCUS_DELAY_MS = 50
const SIDEBAR_COLLAPSED_KEY = 'hq.chat.sidebarCollapsed'
const DESKTOP_QUERY = '(min-width: 768px)'
// A reply alert links here with the chat to open.
const THREAD_PARAM = 'thread'

function readCollapsed() {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === '1'
  } catch {
    return false
  }
}

/** A saved user message being rewritten in the composer; sending replaces it and everything after. */
interface Editing {
  messageId: string
  attachments: ChatAttachment[]
}

interface Props {
  /** Only the visible instance loads data, takes a pending draft and grabs focus. */
  active: boolean
  /** Shown as a close button in the header when set (the overlay). */
  onClose?: () => void
  /** The chat an address names (`/chat/:id`); it opens that chat. */
  threadId?: string
}

/** The whole chat UI: thread list, messages and composer. The /chat route and the global overlay both render it. */
export function ChatView({ active, onClose, threadId }: Props) {
  const chatDraft = useHQStore((s) => s.chatDraft)
  const setChatDraft = useHQStore((s) => s.setChatDraft)
  const clearChatUnread = useHQStore((s) => s.clearChatUnread)
  const threads = useThreadStore((s) => s.threads)
  const activeThreadId = useThreadStore((s) => s.activeThreadId)
  const setActiveThread = useThreadStore((s) => s.setActiveThread)
  const messages = useThreadStore((s) => (activeThreadId ? s.threadMessages[activeThreadId] : undefined)) ?? []
  const liveTurn = useLiveTurn(activeThreadId)
  const { send, connected } = useWS()
  const connState = useSharedConnection()
  const banner = bannerKey(connState, isSharedComputer())
  const outboxChat = useChatOutbox(activeThreadId)

  const [loadingThreads, setLoadingThreads] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readCollapsed)
  const [followSignal, setFollowSignal] = useState(0)
  const [editing, setEditing] = useState<Editing | null>(null)
  const [sessionsOpen, setSessionsOpen] = useState(false)
  const watched = useThreadSessions(activeThreadId, active, sessionsOpen)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const draft = useDraft(activeThreadId)
  const { attachments, addFiles, takeReady } = draft
  const input = draft.text
  const drop = useFileDrop(addFiles)

  const focusInput = () => setTimeout(() => inputRef.current?.focus(), FOCUS_DELAY_MS)

  useEffect(() => {
    if (!active) return
    const linked = threadId ?? new URLSearchParams(window.location.search).get(THREAD_PARAM)
    if (linked) setActiveThread(linked)
    if (linked && !threadId) {
      const url = new URL(window.location.href)
      url.searchParams.delete(THREAD_PARAM)
      window.history.replaceState(null, '', url.toString())
    }
    setLoadingThreads(true)
    refreshThreads()
      .then((list) => {
        const current = useThreadStore.getState().activeThreadId
        // A remembered chat that is gone opens the start screen, not another chat.
        if (current && !list.some((t) => t.threadId === current)) setActiveThread(null)
      })
      .catch((err) => console.warn('ChatView: failed to list threads', err))
      .finally(() => setLoadingThreads(false))
    focusInput()
  }, [active, setActiveThread, threadId])

  useEffect(() => {
    if (!active || !activeThreadId) return
    void loadMessages(activeThreadId).catch((err) => console.warn('ChatView: failed to load messages', err))
  }, [active, activeThreadId])

  // Replies that arrived while nobody was looking stop counting once the chat is on screen.
  useEffect(() => {
    if (!active) return
    const clearIfSeen = () => {
      if (document.visibilityState === 'visible') clearChatUnread()
    }
    clearIfSeen()
    document.addEventListener('visibilitychange', clearIfSeen)
    return () => document.removeEventListener('visibilitychange', clearIfSeen)
  }, [active, clearChatUnread])

  useEffect(() => {
    setEditing(null)
    setSessionsOpen(false)
  }, [activeThreadId])

  // Unwatching the last session hides the button, so the panel goes with it.
  useEffect(() => {
    if (watched.sessions.length === 0) setSessionsOpen(false)
  }, [watched.sessions.length])

  // A note action hands over text; append it so a half-written message survives.
  useEffect(() => {
    if (!active || !chatDraft) return
    draft.setText(input.trim() ? `${input}\n\n${chatDraft}` : chatDraft)
    setChatDraft(null)
  }, [active, chatDraft, setChatDraft])

  const openThread = (id: string) => {
    setActiveThread(id)
    setSidebarOpen(false)
    focusInput()
  }

  const startNewThread = async (title?: string) => {
    try {
      openThread(await newThread(title))
    } catch (err) {
      console.error('ChatView: failed to create a chat', err)
    }
  }

  const archiveThread = useCallback(async (id: string) => {
    try {
      await threadApi.archiveThread(id)
      const next = useThreadStore.getState().threads.filter((t) => t.threadId !== id)
      useThreadStore.getState().setThreads(next)
      if (useThreadStore.getState().activeThreadId === id) setActiveThread(next[0]?.threadId ?? null)
      toastUndo(
        t('undo_chat_archived'),
        async () => {
          await threadApi.unarchiveThread(id)
          await refreshThreads()
        },
        t('restored_chat'),
        'undo.chat_archived',
      )
    } catch (err) {
      console.error('ChatView: failed to archive chat', err)
      toastAlert(failureLine(err, 'fail_archive_chat_newer', t('fail_archive_chat_no_reply')))
    }
  }, [setActiveThread])

  /** `/clear` and `/topic <name>` archive this chat and open a fresh one. */
  const runSlashCommand = async (content: string, tid: string): Promise<boolean> => {
    const parsed = parseSlashCommand(content)
    if (parsed?.command !== '/clear' && parsed?.command !== '/topic') {
      // A lone word after a slash is a command that does not exist; anything longer is ordinary text.
      if (parsed && parsed.args === null && /^\/[a-z]+$/.test(parsed.command)) {
        toastNotice(t('chat_slash_unknown'))
        return true
      }
      return false
    }
    await archiveThread(tid)
    await startNewThread(parsed.command === '/topic' && parsed.args ? parsed.args : undefined)
    return true
  }

  /** Shows the message at once and starts the reply; the server's copy replaces it on turn_start. */
  const sendChat = (tid: string, content: string, files: ChatAttachment[], replaceFrom?: string) => {
    const store = useThreadStore.getState()
    if (replaceFrom) store.removeFrom(tid, replaceFrom)
    // `local-` marks it unsaved here; the random rest keeps two devices from ever sharing one, since HQ drops repeats.
    const clientId = `local-${newClientId()}`
    store.appendMessage(tid, {
      messageId: clientId,
      threadId: tid,
      role: 'user',
      content,
      createdAt: Date.now(),
      attachments: files.length > 0 ? files : undefined,
    })
    store.startTurn(tid)
    setFollowSignal((n) => n + 1)
    const sent = send({ type: 'chat', content, thread_id: tid, attachments: files, client_id: clientId, replace_from: replaceFrom })
    // The socket closed after the connected check: the message never left, so undo it instead of waiting on a reply that cannot come.
    if (!sent) rejectSend(tid, clientId, t(sendFailureKey(connState)), false)
  }

  const handleSend = async (overrideContent?: string) => {
    // With the socket down a plain message waits in the outbox; anything else waits for the socket.
    if (!connected && (await queueOffline(overrideContent))) return
    // A send on a closed socket is dropped, which would leave the chat stuck on "working".
    if (!connected || liveTurn || attachments.some((a) => a.status === 'uploading')) return
    const content = overrideContent ?? input.trim()
    const kept = overrideContent ? [] : editing?.attachments ?? []
    if (!content && kept.length === 0 && !attachments.some((a) => a.status === 'ready')) return
    const files = overrideContent ? [] : [...kept, ...takeReady()]
    const replaceFrom = overrideContent ? undefined : editing?.messageId
    setEditing(null)
    let tid = activeThreadId
    if (!tid) {
      try {
        tid = await newThread()
        setActiveThread(tid)
      } catch (err) {
        console.error('ChatView: failed to create a chat', err)
        return
      }
    }
    if (!replaceFrom && files.length === 0 && isSlashInput(content) && (await runSlashCommand(content, tid))) return
    sendChat(tid, content, files, replaceFrom)
  }

  /** Puts a typed message in "Waiting to send" while HQ cannot be reached. Returns whether it did. */
  const queueOffline = async (overrideContent?: string): Promise<boolean> => {
    const content = overrideContent ?? input.trim()
    if (!outboxChat.enabled || !content || editing || attachments.length > 0 || !activeThreadId) return false
    const refused = await outboxChat.queue(activeThreadId, content)
    if (refused) {
      useHQStore.getState().setSystemNotice(refused)
      return true
    }
    if (!overrideContent) draft.setText('')
    return true
  }

  const startEdit = (m: ThreadMessage) => {
    setEditing({ messageId: m.messageId, attachments: m.attachments ?? [] })
    draft.setText(m.content)
    focusInput()
  }

  /** Runs the last question again: it and the reply after it are replaced. */
  const regenerate = () => {
    if (!activeThreadId || !connected || liveTurn) return
    const question = [...messages].reverse().find((m) => m.role === 'user' && isSaved(m))
    if (question) sendChat(activeThreadId, question.content, question.attachments ?? [], question.messageId)
  }

  const handleStop = () => {
    if (!activeThreadId) return
    void stopReply(activeThreadId, send, threadApi.stopThread).then((ok) => {
      if (!ok) useHQStore.getState().setSystemNotice(t(stopFailureKey(connState)))
    })
  }

  // One button: a drawer on mobile, a persisted collapse on desktop.
  const toggleSidebar = () => {
    if (!window.matchMedia(DESKTOP_QUERY).matches) {
      setSidebarOpen((v) => !v)
      return
    }
    setSidebarCollapsed((v) => {
      try {
        localStorage.setItem(SIDEBAR_COLLAPSED_KEY, v ? '0' : '1')
      } catch {
        // Storage can be blocked; the toggle still works for this session.
      }
      return !v
    })
  }

  const activeThread = threads.find((t) => t.threadId === activeThreadId)

  return (
    <div className="flex h-full w-full text-white overflow-hidden relative">
      <ThreadSidebar
        open={sidebarOpen}
        collapsed={sidebarCollapsed}
        onClose={() => setSidebarOpen(false)}
        loading={loadingThreads}
        onNew={() => void startNewThread()}
        onOpen={openThread}
        onArchive={(id) => void archiveThread(id)}
      />

      <div className="flex-1 flex flex-col h-full min-w-0 relative" {...drop.handlers}>
        {drop.dragging && (
          <div className="absolute inset-2 z-30 rounded-2xl border-2 border-dashed border-emerald-400/60 bg-neutral-950/80 flex flex-col items-center justify-center gap-2 text-sm text-neutral-200 pointer-events-none">
            <Paperclip className="w-6 h-6 text-emerald-400" />
            <span className="font-medium">{t('chat_drop_title')}</span>
            <span className="text-xs text-neutral-400">{t('chat_drop_line')}</span>
          </div>
        )}
        <ChatHeader
          title={activeThread?.title ?? t('chat_new_chat')}
          onToggleSidebar={toggleSidebar}
          onClose={onClose}
          watching={watched.sessions}
          sessionsOpen={sessionsOpen}
          onToggleSessions={() => setSessionsOpen((v) => !v)}
        />
        {sessionsOpen && (
          <SessionsPanel
            sessions={watched.sessions}
            busy={watched.busy}
            onDrive={(id, drive) => void watched.setDrive(id, drive)}
            onUnwatch={(id) => void watched.unwatch(id)}
          />
        )}

        <MessageList
          threadId={activeThreadId}
          messages={messages}
          followSignal={followSignal}
          onEdit={startEdit}
          onRegenerate={regenerate}
          onPick={(text) => draft.setText(text)}
        />

        <OutboxGroup
          entries={outboxChat.entries}
          state={outboxChat.state}
          inHub={false}
          sending={outboxChat.sending}
          textOf={(e) => (e.payload as { content: string }).content}
          onRemove={(id) => void outboxChat.remove(id)}
          onSendNow={(id) => void outboxChat.sendNow(id)}
        />

        {banner && (
          <div role="status" className="px-4 py-1.5 border-t border-amber-500/30 text-[11px] text-amber-400 bg-neutral-900/60">
            {t(banner)}
          </div>
        )}

        {editing && (
          <div className="flex items-center gap-2 px-4 py-1.5 border-t border-white/10 text-[11px] text-neutral-400 bg-neutral-900/60">
            <PencilLine className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
            <span className="flex-1 min-w-0 truncate">
              {t('chat_editing')}. {t('chat_editing_line')}
            </span>
            <button
              type="button"
              onClick={() => {
                setEditing(null)
                draft.setText('')
              }}
              className="flex items-center gap-1 h-8 px-2 rounded hover:bg-white/10 hover:text-white"
            >
              <X className="w-3.5 h-3.5" /> {t('chat_stop_editing')}
            </button>
          </div>
        )}

        <ChatComposer
          ref={inputRef}
          value={input}
          onChange={draft.setText}
          onSend={(content) => void handleSend(content)}
          onStop={handleStop}
          running={!!liveTurn}
          canQueue={outboxChat.enabled && !!activeThreadId && !editing}
          attachments={attachments}
          onAddFiles={addFiles}
          onRemoveAttachment={draft.removeAttachment}
          onRetryAttachment={draft.retryAttachment}
          connected={connected}
          hasKeptFiles={(editing?.attachments.length ?? 0) > 0}
        />
      </div>
    </div>
  )
}
