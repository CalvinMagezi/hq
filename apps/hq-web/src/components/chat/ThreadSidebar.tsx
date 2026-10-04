import { useMemo, useState } from 'react'
import { Plus, MessageSquare, Trash2, Search, X, Loader2 } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { relTime } from '~/lib/time'

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
        className={`fixed md:relative inset-y-0 left-0 z-50 md:z-auto w-72 shrink-0 border-r border-white/10 flex flex-col h-full pad-safe-top pb-[var(--safe-bottom)] md:pb-0 bg-neutral-900/95 md:bg-neutral-900/60 backdrop-blur-xl transition-transform duration-200 ${
          open ? 'translate-x-0' : '-translate-x-full md:translate-x-0'
        } ${collapsed ? 'md:hidden' : ''}`}
      >
        <div className="p-3 border-b border-white/10 flex items-center justify-between gap-2">
          <button
            type="button"
            onClick={onNew}
            className="flex-1 flex items-center justify-center gap-2 py-2 px-3 rounded-xl bg-emerald-500/15 border border-emerald-500/30 text-emerald-400 font-bold text-sm hover:bg-emerald-500/25 transition-all"
          >
            <Plus className="w-4 h-4" /> New chat
            {runningCount > 0 && <span className="text-[11px] font-normal text-emerald-300/70">({runningCount} running)</span>}
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
              placeholder="Search chats..."
              className="w-full pl-8 pr-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-sm outline-none focus:border-emerald-500/40"
            />
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {loading && threads.length === 0 && <div className="text-center py-6 text-xs text-neutral-500">Loading chats...</div>}
          {!loading && filteredThreads.length === 0 && <div className="text-center py-6 text-xs text-neutral-500">No chats yet</div>}

          {filteredThreads.map((t) => {
            const isActive = t.threadId === activeThreadId
            return (
              <div
                key={t.threadId}
                onClick={() => onOpen(t.threadId)}
                className={`group flex items-center justify-between p-2.5 rounded-xl cursor-pointer transition-all ${
                  isActive ? 'bg-emerald-500/15 border border-emerald-500/30' : 'hover:bg-white/5 border border-transparent'
                }`}
              >
                <div className="flex items-center gap-2.5 overflow-hidden">
                  {t.threadId in running ? (
                    <Loader2 className="w-4 h-4 shrink-0 animate-spin text-emerald-400" aria-label="Running" />
                  ) : (
                    <MessageSquare className={`w-4 h-4 shrink-0 ${isActive ? 'text-emerald-400' : 'text-neutral-400'}`} />
                  )}
                  <div className="truncate text-sm">
                    <div className="flex items-center gap-1.5">
                      <span className={`font-semibold truncate ${isActive ? 'text-emerald-300' : 'text-neutral-200'}`}>{t.title}</span>
                      {t.unreadCount > 0 && !isActive && (
                        <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 shrink-0" aria-label="New reply" />
                      )}
                    </div>
                    <div className="text-xs text-neutral-500 truncate">
                      {relTime(t.updatedAt)}
                      {t.lastMessagePreview ? ` · ${t.lastMessagePreview}` : ''}
                    </div>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation()
                    onArchive(t.threadId)
                  }}
                  className="opacity-0 group-hover:opacity-100 p-1 text-neutral-500 hover:text-red-400 transition-all"
                  title="Archive chat"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            )
          })}
        </div>
      </div>
    </>
  )
}
