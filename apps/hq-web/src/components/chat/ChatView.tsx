import { useCallback, useEffect, useRef, useState } from 'react'
import { newClientId } from '~/lib/outbox'
import { ChevronLeft, Info, Paperclip, PencilLine, X } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { useThreadStore, useLiveTurn, type ThreadMessage } from '~/store/threadStore'
import { useWS } from '~/context/WebSocketContext'
import { toastUndo, toastAlert, toastNotice } from '~/lib/toast'
import { failureLine } from '~/lib/version'
import { t } from '~/strings'
import { sharedConnectionNow, useSharedConnection } from '~/lib/connection'
import { isSharedComputer } from '~/lib/sharedComputer'
import { archiveChatKey } from '~/lib/failureWords'
import { useInHub } from '~/lib/hubWindow'
import { bannerKey, savedBannerLine, sendFailureKey, stopFailureKey, stopReply } from '~/lib/chatConnection'
import { threadApi, refreshThreads, loadMessages, newThread } from '~/lib/threadApi'
import { rejectSend } from '~/lib/chatEvents'
import { answerOffer } from '~/lib/offersApi'
import { MAX_ATTACHMENTS, type ChatAttachment } from '~/lib/chatUploads'
import { parseSlashCommand, isSlashInput } from './SlashCommandPalette'
import { ChatList, NewChatButton } from './ChatList'
import { Panes } from '../ui/Panes'
import { Sheet } from '../ui/Sheet'
import { ChatComposer } from './ChatComposer'
import { MessageList, isSaved } from './MessageList'
import { OutboxGroup } from './OutboxGroup'
import { useChatOutbox } from './useChatOutbox'
import { useDraft } from './draftStore'
import { useFileDrop } from './useFileDrop'
import { useThreadSessions } from './useThreadSessions'
import { SessionsPanel } from './SessionsPanel'
import { ChatApprovals } from './ChatApprovals'

const FOCUS_DELAY_MS = 50
// A reply alert links here with the chat to open.
const THREAD_PARAM = 'thread'
// From here the details pane is docked and always shown.
const DOCKED_QUERY = '(min-width: 1400px)'

/** A saved user message being rewritten in the composer; sending replaces it and everything after. */
interface Editing {
  messageId: string
  attachments: ChatAttachment[]
}

interface Props {
  /** Only the visible instance loads data, takes a pending draft and grabs focus. */
  active: boolean
  /** The chat an address names (`/chat/:id`); it opens that chat. */
  threadId?: string
}

/** The whole chat (AlignedChat): the chats list, the open chat and its details, on the shared panes. */
export function ChatView({ active, threadId }: Props) {
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
  const inHub = useInHub()
  const savedLine = savedBannerLine(connState, isSharedComputer(), inHub, messages.reduce<number | null>((m, x) => (m === null || x.createdAt > m ? x.createdAt : m), null))
  const outboxChat = useChatOutbox(activeThreadId)
  const waitingForOwner = useThreadStore((s) => (activeThreadId ? s.waiting[activeThreadId] === true : false))

  const [loadingThreads, setLoadingThreads] = useState(false)
  // Below 1024 px the chats list is a sheet; from 1024 it is the list pane.
  const [listOpen, setListOpen] = useState(false)
  // Below 1400 px the details lie over the chat while open; from 1400 they are docked.
  const [sideOpen, setSideOpen] = useState(false)
  const [followSignal, setFollowSignal] = useState(0)
  const [editing, setEditing] = useState<Editing | null>(null)
  const [sessionsOpen, setSessionsOpen] = useState(false)
  const watched = useThreadSessions(activeThreadId, active, sessionsOpen)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const draft = useDraft(activeThreadId)
  const { attachments, addFiles, takeReady } = draft
  const input = draft.text
  const drop = useFileDrop(addFiles)

  const focusInput = () => setTimeout(() => inputRef.current?.focus({ preventScroll: true }), FOCUS_DELAY_MS)

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
  }, [activeThreadId])

  // The watched helpers are asked for only while their details can be seen.
  useEffect(() => {
    setSessionsOpen(sideOpen || (typeof window !== 'undefined' && window.matchMedia(DOCKED_QUERY).matches))
  }, [sideOpen])

  // A note action hands over text; append it so a half-written message survives.
  useEffect(() => {
    if (!active || !chatDraft) return
    draft.setText(input.trim() ? `${input}\n\n${chatDraft}` : chatDraft)
    setChatDraft(null)
  }, [active, chatDraft, setChatDraft])

  const openThread = (id: string) => {
    setActiveThread(id)
    setListOpen(false)
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
      toastAlert(failureLine(err, 'fail_archive_chat_newer', t(archiveChatKey(sharedConnectionNow()))))
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

  /** Yes sends the offer's request, or puts it in the composer when it needs the person's words. */
  const answerOfferOn = (m: ThreadMessage, yes: boolean) => {
    const offer = m.offer
    if (!offer || !activeThreadId) return
    void answerOffer(activeThreadId, m.messageId, yes ? 'yes' : 'not_now').catch((err) =>
      console.warn('ChatView: could not record the answer to an offer', err),
    )
    if (!yes) return
    if (offer.send && connected && !liveTurn) {
      sendChat(activeThreadId, offer.reply, [])
      return
    }
    draft.setText(offer.reply)
    focusInput()
  }

  const handleStop = () => {
    if (!activeThreadId) return
    void stopReply(activeThreadId, send, threadApi.stopThread).then((ok) => {
      if (!ok) useHQStore.getState().setSystemNotice(t(stopFailureKey(connState)))
    })
  }

  const activeThread = threads.find((t) => t.threadId === activeThreadId)

  const chatList = (head: boolean) => (
    <ChatList
      loading={loadingThreads}
      onNew={() => {
        setListOpen(false)
        void startNewThread()
      }}
      onOpen={openThread}
      onArchive={(id) => void archiveThread(id)}
      withHead={head}
    />
  )

  const title = activeThread?.title ?? t('chat_new_chat')
  const detail = (
    <div className="hq-chat-detail" {...drop.handlers}>
      {drop.dragging && (
        <div className="hq-chat-drop" aria-hidden="true">
          <Paperclip className="a-ic" />
          <b>{t('chat_drop_title')}</b>
          <span className="a-body-sm a-muted">{t('chat_drop_line')}</span>
        </div>
      )}
      <div className="a-pane-head">
        <button type="button" className="a-ib a-back" aria-label={t('chat_chats')} onClick={() => setListOpen(true)}>
          <ChevronLeft className="a-ic" aria-hidden="true" />
        </button>
        <h2 className="a-h3">{title}</h2>
        <button type="button" className="a-ib sm a-side-toggle" aria-label={t('chat_details')} aria-expanded={sideOpen} onClick={() => setSideOpen((v) => !v)}>
          <Info className="a-ic sm" aria-hidden="true" />
        </button>
      </div>

      <MessageList
        threadId={activeThreadId}
        messages={messages}
        followSignal={followSignal}
        onEdit={startEdit}
        onRegenerate={regenerate}
        onPick={(text) => draft.setText(text)}
        onOffer={answerOfferOn}
      />

      <ChatApprovals threadId={activeThreadId} waiting={waitingForOwner} onKeepGoing={() => void handleSend(t('chat_keep_going'))} />

      <OutboxGroup
        entries={outboxChat.entries}
        state={outboxChat.state}
        inHub={inHub}
        sending={outboxChat.sending}
        textOf={(e) => (e.payload as { content: string }).content}
        onRemove={(id) => void outboxChat.remove(id)}
        onSendNow={(id) => void outboxChat.sendNow(id)}
      />

      {banner && (
        <div role="status" className="a-stale">
          <span>
            {t(banner)}
            {savedLine && <span className="hq-block">{savedLine}</span>}
          </span>
        </div>
      )}

      {editing && (
        <div className="a-stale">
          <PencilLine className="a-ic sm" aria-hidden="true" />
          <span>
            {t('chat_editing')}. {t('chat_editing_line')}
          </span>
          <button
            type="button"
            className="a-btn quiet compact"
            onClick={() => {
              setEditing(null)
              draft.setText('')
            }}
          >
            <X className="a-ic sm" aria-hidden="true" />
            {t('chat_stop_editing')}
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
  )

  const side = (
    <>
      <div className="a-pane-head">
        <h2 className="a-h3">{t('chat_details')}</h2>
        <button type="button" className="a-ib sm a-side-toggle" aria-label={t('close')} onClick={() => setSideOpen(false)}>
          <X className="a-ic sm" aria-hidden="true" />
        </button>
      </div>
      <SessionsPanel
        sessions={watched.sessions}
        busy={watched.busy}
        onDrive={(id, drive) => void watched.setDrive(id, drive)}
        onUnwatch={(id) => void watched.unwatch(id)}
        onArchive={activeThreadId ? () => void archiveThread(activeThreadId) : undefined}
      />
    </>
  )

  return (
    <>
      <Panes
        list={chatList(true)}
        listLabel={t('chat_chats')}
        detail={detail}
        detailLabel={title}
        side={side}
        sideLabel={t('chat_details')}
        showDetail
        sideOpen={sideOpen}
      />
      {listOpen && (
        <Sheet
          title={t('chat_chats')}
          placement="bottom"
          className="hq-chats-sheet"
          onClose={() => setListOpen(false)}
          headActions={<NewChatButton onNew={() => { setListOpen(false); void startNewThread() }} />}
        >
          {chatList(false)}
        </Sheet>
      )}
    </>
  )
}
