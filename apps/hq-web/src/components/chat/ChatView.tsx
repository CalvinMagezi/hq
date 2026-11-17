import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { newClientId } from '~/lib/outbox'
import { ChevronLeft, PanelRight, Paperclip, PencilLine, X } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { useThreadStore, useLiveTurn, type ThreadMessage } from '~/store/threadStore'
import { useWS } from '~/context/WebSocketContext'
import { toastUndo, toastAlert, toastNotice, useToasts } from '~/lib/toast'
import { makeTaskDraft, saveReplyToVault } from '~/lib/saveReply'
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
import { SessionsPanel, DrivingStrip } from './SessionsPanel'
import { ChatApprovals } from './ChatApprovals'
import { OfferLine } from './OfferLine'
import { ChatTitle } from './ChatTitle'
import { WorkPane } from './WorkPane'
import { useWorkPane, usePane } from '~/store/workPaneStore'
import { madeNotes, type MadeNote, type ReplySource } from '~/lib/replySources'
import { readAutoOpened, shouldAutoOpen, writeAutoOpened } from '~/lib/workPane'
import { readPaneMode } from '~/lib/paneMode'
import { useQuery } from '@tanstack/react-query'
import { hasViewer, readHqmd } from '~/lib/hqmd'

const FOCUS_DELAY_MS = 50
// A reply alert links here with the chat to open.
const THREAD_PARAM = 'thread'
// "Ask HQ", "New chat" and a note's Ask HQ link here to start a new chat.
const COMPOSE_PARAM = 'compose'
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
  const navigate = useNavigate()
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
  // The work pane lies over the chat below 1400 px; from 1400 it sits beside it.
  const [docked, setDocked] = useState(false)
  const pane = usePane(activeThreadId)
  const panes = useWorkPane.getState
  // A sheet, slides or dashboard needs more room than a note: the pane widens for it, from the copy already loaded.
  const shownNote = useQuery({ queryKey: ['pane-note', pane.note], enabled: false })
  const wideKind = pane.open && pane.tab === 'result' && typeof (shownNote.data as { content?: string } | undefined)?.content === 'string' && hasViewer(readHqmd((shownNote.data as { content: string }).content))
  const [followSignal, setFollowSignal] = useState(0)
  const [editing, setEditing] = useState<Editing | null>(null)
  const sessionsOpen = pane.open && pane.tab === 'helpers'
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
    // "Ask HQ" and "New chat" link here with compose=1: the start screen, where their words wait in the box.
    // The router writes its search values as JSON, so the 1 may come quoted.
    const compose = !linked && new URLSearchParams(window.location.search).get(COMPOSE_PARAM)?.replace(/"/g, '') === '1'
    if (compose) {
      setActiveThread(null)
      const url = new URL(window.location.href)
      url.searchParams.delete(COMPOSE_PARAM)
      window.history.replaceState(null, '', url.toString())
    }
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

  // Whether the pane has room beside the chat; below that it lies over it.
  useEffect(() => {
    const query = window.matchMedia(DOCKED_QUERY)
    const update = () => setDocked(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])

  // The pane opens by itself once per chat, when HQ finishes making a note, and only when it has room.
  const liveMade = madeNotes((liveTurn?.toolSteps ?? []).filter((x) => x.status === 'done')).at(-1)?.path ?? null
  useEffect(() => {
    if (!active || !activeThreadId) return
    const opened = readAutoOpened()
    // The Settings choice: only "automatic" opens the pane by itself.
    if (readPaneMode() !== 'auto') return
    if (!shouldAutoOpen({ made: liveMade, docked, already: opened.includes(activeThreadId) })) return
    writeAutoOpened([...opened, activeThreadId])
    if (liveMade) panes().showNote(activeThreadId, liveMade)
  }, [active, activeThreadId, liveMade, docked, panes])

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

  /** Keeps a reply as a note. The toast says so, offers to open it, and Undo puts it in the Trash. */
  const saveReply = (m: ThreadMessage) => {
    void saveReplyToVault(m.content, m.messageId)
      .then(({ path, undo }) =>
        useToasts.getState().push({
          text: t('chat_saved_vault'),
          kind: 'status',
          undo,
          restored: t('chat_save_removed'),
          action: { label: t('chat_saved_open'), run: () => void navigate({ to: '/notes/$', params: { _splat: path } }) },
        }),
      )
      .catch(() => toastAlert(t('chat_save_failed')))
  }

  /** A task is HQ's to make, so the request goes in the composer for the person to read and send. */
  const makeTaskFrom = (m: ThreadMessage) => {
    draft.setText(makeTaskDraft(m.content))
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

  /** The line goes at once; the person is told whether it was scheduled. */
  const scheduleOfferOn = (m: ThreadMessage, at: number) => {
    if (!m.offer || !activeThreadId) return
    const notice = useHQStore.getState().setSystemNotice
    void answerOffer(activeThreadId, m.messageId, 'scheduled', at)
      .then(() => notice(t('chat_offer_scheduled')))
      .catch((err) => {
        console.warn('ChatView: could not schedule an offer', err)
        notice(t('chat_offer_schedule_failed'))
      })
  }

  const handleStop = () => {
    if (!activeThreadId) return
    void stopReply(activeThreadId, send, threadApi.stopThread).then((ok) => {
      if (!ok) useHQStore.getState().setSystemNotice(t(stopFailureKey(connState)))
    })
  }

  /** The person names the chat. It shows at once; if HQ refuses, the old name comes back. */
  const renameChat = (next: string) => {
    if (!activeThreadId) return
    const id = activeThreadId
    const before = threads.find((x) => x.threadId === id)?.title
    useThreadStore.getState().updateThread(id, { title: next })
    threadApi.renameThread(id, next).catch(() => {
      if (before !== undefined) useThreadStore.getState().updateThread(id, { title: before })
      toastAlert(t('chat_rename_failed'))
    })
  }

  const openMade = (note: MadeNote) => {
    // "Never" in Settings: a card opens the note as a page in the Vault.
    if (readPaneMode() === 'never') void navigate({ to: '/notes/$', params: { _splat: note.path } })
    else if (activeThreadId) panes().showNote(activeThreadId, note.path)
  }
  const openSource = (source: ReplySource) => {
    if (activeThreadId) panes().showSource(activeThreadId, source.ref)
  }
  /** Ask HQ about a note the pane shows: its path goes in the box, after anything already typed. */
  const askAbout = (path: string) => {
    draft.setText(input.trim() ? `${input}\n\n\`${path}\`` : `\`${path}\``)
    focusInput()
  }
  const taskFromNote = (path: string) => {
    draft.setText(t('chat_pane_task_draft', { path }))
    focusInput()
  }
  const lastAssistant = [...messages].reverse().find((m) => m.role === 'assistant')
  const contextSteps = liveTurn?.toolSteps ?? lastAssistant?.toolSteps ?? []
  const contextCredits = liveTurn?.stepCredits ?? lastAssistant?.stepCredits

  /** The last saved message of mine goes back in the box to be changed and sent again. */
  const editLast = () => {
    const mine = [...messages].reverse().find((m) => m.role === 'user' && isSaved(m))
    if (mine && !liveTurn) startEdit(mine)
  }

  // Keys for the whole chat: the pane, the neighbouring chats and the last reply.
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey
      if (mod && !e.shiftKey && !e.altKey && e.key === '.') {
        e.preventDefault()
        if (activeThreadId) panes().toggle(activeThreadId)
        return
      }
      if (mod && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'c') {
        const reply = [...useThreadStore.getState().threadMessages[activeThreadId ?? ''] ?? []].reverse().find((m) => m.role === 'assistant')
        // Selected text keeps the browser's own copy.
        if (reply && !window.getSelection()?.toString()) {
          e.preventDefault()
          void navigator.clipboard.writeText(reply.content)
        }
        return
      }
      if (e.altKey && !mod && !e.shiftKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
        const list = useThreadStore.getState().threads
        const at = list.findIndex((x) => x.threadId === activeThreadId)
        const to = list[at + (e.key === 'ArrowUp' ? -1 : 1)]
        if (to) {
          e.preventDefault()
          openThread(to.threadId)
        }
      }
    }
    // The desktop app's menu sends this for Work pane, since its key never reaches the page.
    const onTogglePane = () => {
      if (activeThreadId) panes().toggle(activeThreadId)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('hq:toggle-pane', onTogglePane)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('hq:toggle-pane', onTogglePane)
    }
  })

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
  // The one slot above the box: an approval wins it, then an offer. Nothing offers anything while a reply streams.
  const offerOn = !liveTurn && messages.length > 0 && messages[messages.length - 1].offer ? messages[messages.length - 1] : null
  const slotOffer = offerOn?.offer ? (
    <OfferLine offer={offerOn.offer} onAnswer={(yes) => answerOfferOn(offerOn, yes)} onSchedule={(at) => scheduleOfferOn(offerOn, at)} />
  ) : null
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
        <ChatTitle title={title} onRename={activeThreadId ? renameChat : undefined} />
        <button
          type="button"
          className="a-ib sm"
          aria-label={t('chat_pane_toggle')}
          aria-pressed={pane.open}
          aria-controls="hq-pane-panel"
          disabled={!activeThreadId}
          onClick={() => activeThreadId && panes().toggle(activeThreadId)}
        >
          <PanelRight className="a-ic sm" aria-hidden="true" />
        </button>
      </div>
      <DrivingStrip sessions={watched.sessions} onOpen={() => activeThreadId && panes().showTab(activeThreadId, 'helpers')} />

      <MessageList
        threadId={activeThreadId}
        messages={messages}
        followSignal={followSignal}
        onEdit={startEdit}
        onRegenerate={regenerate}
        onSave={saveReply}
        onMakeTask={makeTaskFrom}
        onPick={(text) => draft.setText(text)}
        onStop={handleStop}
        onOpenSource={openSource}
        onOpenMade={openMade}
      />

      <ChatApprovals threadId={activeThreadId} waiting={waitingForOwner} onKeepGoing={() => void handleSend(t('chat_keep_going'))} otherwise={slotOffer} />

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
        onEditLast={editLast}
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

  const side = activeThreadId ? (
    <WorkPane
      pane={pane}
      onTab={(tab) => panes().showTab(activeThreadId, tab)}
      onClose={() => panes().close(activeThreadId)}
      steps={contextSteps}
      startedAt={liveTurn?.startedAt}
      credits={contextCredits}
      running={!!liveTurn}
      onSelectSource={(ref) => panes().showSource(activeThreadId, ref)}
      onAsk={askAbout}
      onTask={taskFromNote}
      helpers={
        <SessionsPanel
          sessions={watched.sessions}
          busy={watched.busy}
          onDrive={(id, drive) => void watched.setDrive(id, drive)}
          onUnwatch={(id) => void watched.unwatch(id)}
          onArchive={() => void archiveThread(activeThreadId)}
          threadId={activeThreadId}
        />
      }
    />
  ) : undefined

  return (
    <>
      <Panes
        list={chatList(true)}
        listLabel={t('chat_chats')}
        detail={detail}
        detailLabel={title}
        side={pane.open ? side : undefined}
        sideLabel={t('chat_pane_toggle')}
        showDetail
        sideOpen={pane.open}
        className={wideKind ? 'hq-chat-panes hq-pane-wide' : 'hq-chat-panes'}
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
