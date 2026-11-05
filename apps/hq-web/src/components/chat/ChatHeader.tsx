import { Eye, PanelLeft, X } from 'lucide-react'
import { t } from '~/strings'
import { needsAttention, watchingLabel, type WatchedSession } from '~/lib/sessionsApi'

interface Props {
  title: string
  onToggleSidebar: () => void
  /** Shown as a close button when set (the overlay). */
  onClose?: () => void
  /** Coding-agent sessions this chat watches; the button hides when there are none. */
  watching?: WatchedSession[]
  sessionsOpen?: boolean
  onToggleSessions?: () => void
}

/** Chat list toggle, the chat's title, watched sessions, and, in the overlay, close. */
export function ChatHeader({ title, onToggleSidebar, onClose, watching = [], sessionsOpen = false, onToggleSessions }: Props) {
  return (
    <div className="flex items-center gap-1 border-b border-white/10 px-1 md:px-3 shrink-0">
      <button type="button" onClick={onToggleSidebar} className="p-2.5 text-neutral-400 hover:text-white" title={t('chat_toggle_list')} aria-label={t('chat_toggle_list')}>
        <PanelLeft className="w-4 h-4" />
      </button>
      <span className="flex-1 text-sm text-neutral-300 truncate py-2.5">{title}</span>
      {watching.length > 0 && onToggleSessions && (
        <WatchingButton sessions={watching} open={sessionsOpen} onToggle={onToggleSessions} />
      )}
      {onClose && (
        <>
          <kbd className="hidden sm:inline text-[11px] text-neutral-400 bg-white/5 px-2 py-0.5 rounded border border-white/5">⌘K</kbd>
          <button
            type="button"
            onClick={onClose}
            className="p-2.5 rounded-lg hover:bg-white/10 text-neutral-400 hover:text-white transition-colors"
            title={t('chat_close')} aria-label={t('chat_close')}
          >
            <X className="w-4 h-4" />
          </button>
        </>
      )}
    </div>
  )
}

/** Opens the Watching panel. On a phone it is an icon and counts, so its name is spelled out for assistive tech. */
function WatchingButton({ sessions, open, onToggle }: { sessions: WatchedSession[]; open: boolean; onToggle: () => void }) {
  const waiting = sessions.filter(needsAttention).length
  return (
    <button
      type="button"
      onClick={onToggle}
      className={`flex items-center gap-1.5 h-11 min-w-11 sm:h-8 sm:min-w-0 px-2 rounded-lg text-xs hover:bg-white/10 transition-colors shrink-0 ${open ? 'text-white bg-white/5' : 'text-neutral-400 hover:text-white'}`}
      title={t('chat_watching_title')}
      aria-label={watchingLabel(sessions)}
      aria-expanded={open}
    >
      <Eye className="w-4 h-4" />
      <span className="hidden sm:inline">{t('chat_watching')}</span> {sessions.length}
      {waiting > 0 && (
        <span className="flex items-center gap-1" style={{ color: 'var(--accent-amber)' }} aria-hidden="true">
          <span className="w-1.5 h-1.5 rounded-full bg-current" />
          {waiting}
        </span>
      )}
    </button>
  )
}
