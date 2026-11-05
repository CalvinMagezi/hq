import { useMemo, useState } from 'react'
import { Plus, MessageSquare, Archive, Search, X, Loader2 } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { relTime } from '~/lib/time'
import { t } from '~/strings'
import { groupChats } from '~/lib/chatGroups'

interface Props {
  /** Drawer state on mobile; the sidebar is always shown from md up. */
  open: boolean
  /** Hides the sidebar from md up; mobile uses `open` instead. */
  collapsed: boolean
  onClose: () => void
  loading: boolean
  onNew: () => void
  onOpen: (threadId: string) => void
  onArchive: (threadId: string) => void
}

/** Searchable chat list with running and unread markers. */
export function ThreadSidebar({ open, collapsed, onClose, loading, onNew, onOpen, onArchive }: Props) {
  const threads = useThreadStore((s) => s.threads)
  const running = useThreadStore((s) => s.live)
  const activeThreadId = useThreadStore((s) => s.activeThreadId)
  const [searchQuery, setSearchQuery] = useState('')

  const filteredThreads = useMemo(() => {
    const q = searchQuery.trim().toLowerCase()
    if (!q) return threads
    return threads.filter((t) => t.title.toLowerCase().includes(q) || t.lastMessagePreview?.toLowerCase().includes(q))
  }, [threads, searchQuery])
  const runningCount = Object.keys(running).length

  return (
    <>
      {open && <div className="md:hidden fixed inset-0 z-40 bg-black/60 backdrop-blur-sm" onClick={onClose} />}

      <div
        className={`fixed md:relative inset-y-0 left-0 z-50 md:z-auto w-72 shrink-0 border-r border-white/10 flex flex-col h-full pad-safe-top pb-[var(--safe-bottom)] md:pb-0 bg-neutral-950/90 md:bg-transparent backdrop-blur-xl transition-transform duration-200 ${
          open ? 'translate-x-0' : '-translate-x-full md:translate-x-0'
        } ${collapsed ? 'md:hidden' : ''}`}
      >
        <div className="p-3 border-b border-white/10 flex items-center justify-between gap-2">
          <button
            type="button"
            onClick={onNew}
            className="flex-1 flex items-center justify-center gap-2 hq-btn-primary !h-10"
          >
            <Plus className="w-4 h-4" /> {t('chat_new_chat')}
            {runningCount > 0 && <span className="text-[11px] font-normal text-emerald-300/70">({t('chat_row_running', { n: runningCount })})</span>}
          </button>
          <button type="button" onClick={onClose} className="md:hidden p-2 text-neutral-400 hover:text-white">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="p-2 border-b border-white/10">
          <div className="relative">
            <Search className="w-3.5 h-3.5 absolute left-2.5 top-2.5 text-neutral-400" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t('chat_search_chats')}
              aria-label={t('chat_search_chats')}
              className="w-full pl-8 pr-3 py-1.5 rounded-lg hq-field text-sm outline-none"
            />
          </div>
        </div>

        <nav aria-label={t('chat_chats')} className="flex-1 overflow-y-auto p-2 space-y-1">
          {loading && threads.length === 0 && <div className="text-center py-6 text-xs text-neutral-500">{t('chat_list_loading')}</div>}
          {!loading && filteredThreads.length === 0 && (
            <div className="text-center py-6 px-3 text-xs text-neutral-500">
              <p className="font-medium text-neutral-300">{t('chat_list_empty_title')}</p>
              <p className="mt-1">{t('chat_list_empty_line')}</p>
            </div>
          )}

          {groupChats(filteredThreads).map((group) => (
            <section key={group.key}>
              <h2 className="px-2.5 pt-2 pb-1 text-[11px] font-medium uppercase tracking-wide text-neutral-500">{t(group.label)}</h2>
              {group.items.map((th) => {
                const isActive = th.threadId === activeThreadId
                return (
                  <div key={th.threadId} className={`group flex items-center justify-between rounded-xl transition-all ${isActive ? 'hq-row is-selected' : 'hq-row'}`}>
                    <a
                      href={`/chat/${encodeURIComponent(th.threadId)}`}
                      aria-current={isActive ? 'page' : undefined}
                      onClick={(e) => {
                        // A modified click keeps the browser's own open-in-new-tab.
                        if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return
                        e.preventDefault()
                        onOpen(th.threadId)
                      }}
                      className="flex-1 min-w-0 flex items-center gap-2.5 p-2.5 min-h-11"
                    >
                      {th.threadId in running ? (
                        <Loader2 className="w-4 h-4 shrink-0 animate-spin" style={{ color: 'var(--brand-sky)' }} aria-label={t('chat_row_working')} />
                      ) : (
                        <MessageSquare className="w-4 h-4 shrink-0" style={{ color: isActive ? 'var(--brand-blue)' : 'var(--ink-muted)' }} />
                      )}
                      <div className="truncate text-sm">
                        <div className="flex items-center gap-1.5">
                          <span className={`font-semibold truncate ${isActive ? 'text-white' : 'text-neutral-200'}`}>{th.title}</span>
                          {th.unreadCount > 0 && !isActive && (
                            <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: 'var(--brand-blue)' }} aria-label={t('chat_row_unread')} />
                          )}
                        </div>
                        <div className="text-xs text-neutral-500 truncate">
                          {relTime(th.updatedAt)}
                          {th.lastMessagePreview ? ` · ${th.lastMessagePreview}` : ''}
                        </div>
                      </div>
                    </a>
                    <button
                      type="button"
                      onClick={() => onArchive(th.threadId)}
                      className="opacity-60 md:opacity-0 group-hover:opacity-100 focus-visible:opacity-100 min-w-11 min-h-11 flex items-center justify-center hover:text-white"
                      style={{ color: 'var(--ink-muted)' }}
                      title={t('chat_row_archive')}
                      aria-label={t('chat_row_archive')}
                    >
                      <Archive className="w-4 h-4" />
                    </button>
                  </div>
                )
              })}
            </section>
          ))}
        </nav>
      </div>
    </>
  )
}
