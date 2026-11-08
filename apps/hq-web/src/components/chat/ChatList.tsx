import { useMemo, useState } from 'react'
import { Archive, Plus, Search } from 'lucide-react'
import { useThreadStore, type Thread } from '~/store/threadStore'
import { relTime } from '~/lib/time'
import { t } from '~/strings'
import { groupChats, isSpokenThread } from '~/lib/chatGroups'
import './chat.css'

interface Props {
  loading: boolean
  onNew: () => void
  onOpen: (threadId: string) => void
  onArchive: (threadId: string) => void
  /** The list's own head (the pane); a sheet brings its own. */
  withHead?: boolean
}

/** The chats, as the design's list (AlignedChat): search, day groups, rows with the time or "Working". */
export function ChatList({ loading, onNew, onOpen, onArchive, withHead = true }: Props) {
  const threads = useThreadStore((s) => s.threads)
  const running = useThreadStore((s) => s.live)
  const saved = useThreadStore((s) => s.threadMessages)
  const activeThreadId = useThreadStore((s) => s.activeThreadId)
  const [query, setQuery] = useState('')

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return threads
    return threads.filter((th) => th.title.toLowerCase().includes(q) || th.lastMessagePreview?.toLowerCase().includes(q))
  }, [threads, query])

  return (
    <div className="hq-chat-list">
      {withHead && (
        <div className="a-pane-head">
          <h2 className="a-h3">{t('chat_chats')}</h2>
          <NewChatButton onNew={onNew} />
        </div>
      )}
      <div className="hq-chat-list-find">
        <div className="a-search">
          <label className="a-sr" htmlFor="hq-chat-find">
            {t('chat_search_chats')}
          </label>
          <Search className="a-ic sm" aria-hidden="true" />
          <input id="hq-chat-find" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('chat_search_chats')} />
        </div>
      </div>
      <nav className="hq-chat-list-rows" aria-label={t('chat_chats')}>
        {loading && threads.length === 0 && <p className="a-body-sm a-muted hq-chat-empty">{t('chat_list_loading')}</p>}
        {!loading && shown.length === 0 && (
          <div className="hq-chat-empty">
            <p className="a-body-b">{t('chat_list_empty_title')}</p>
            <p className="a-body-sm a-muted">{t('chat_list_empty_line')}</p>
          </div>
        )}
        {groupChats(shown).map((group) => (
          <section key={group.key}>
            <p className="a-grp">{t(group.label)}</p>
            {group.items.map((th) => (
              <ChatRow
                key={th.threadId}
                thread={th}
                active={th.threadId === activeThreadId}
                working={th.threadId in running}
                spoken={isSpokenThread(saved[th.threadId])}
                onOpen={onOpen}
                onArchive={onArchive}
              />
            ))}
          </section>
        ))}
      </nav>
    </div>
  )
}

export function NewChatButton({ onNew }: { onNew: () => void }) {
  return (
    <button type="button" className="a-btn compact" onClick={onNew}>
      <Plus className="a-ic sm" aria-hidden="true" />
      {t('chat_new_chat')}
    </button>
  )
}

interface RowProps {
  thread: Thread
  active: boolean
  working: boolean
  spoken: boolean
  onOpen: (threadId: string) => void
  onArchive: (threadId: string) => void
}

/** One chat in the list: its title and last line, the time or "Working", unread, and archive (on hover, or always on a touch screen). */
export function ChatRow({ thread: th, active, working, spoken, onOpen, onArchive }: RowProps) {
  const unread = !active && th.unreadCount !== 0
  const line = [spoken ? t('chat_spoken') : null, th.lastMessagePreview].filter(Boolean).join(' · ')
  return (
    <div className="hq-chatrow">
      <a
        className="a-row"
        href={`/chat/${encodeURIComponent(th.threadId)}`}
        aria-current={active ? 'page' : undefined}
        onClick={(e) => {
          // A modified click keeps the browser's own open-in-new-tab.
          if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return
          e.preventDefault()
          onOpen(th.threadId)
        }}
      >
        <span className="a-row-main">
          <b>{th.title}</b>
          {line && <span>{line}</span>}
        </span>
        <span className="a-row-trail">
          {working ? (
            <span className="a-state working">
              <span className="a-spin" aria-hidden="true" />
              {t('chat_row_working')}
            </span>
          ) : (
            relTime(th.updatedAt)
          )}
          {unread && <span className="a-unread" aria-label={t('chat_row_unread')} />}
        </span>
      </a>
      <button type="button" className="a-ib sm hq-chatrow-archive" aria-label={t('chat_row_archive_named', { title: th.title })} onClick={() => onArchive(th.threadId)}>
        <Archive className="a-ic sm" aria-hidden="true" />
      </button>
    </div>
  )
}
