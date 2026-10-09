import { useEffect, useState } from 'react'
import { Bell, BellOff, Eye, PanelLeft, X } from 'lucide-react'
import { alertsEnabled, alertsSupported, setAlertsEnabled } from '~/lib/replyAlerts'
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

/** Chat list toggle, the chat's title, watched sessions, the reply-alert switch and, in the overlay, close. */
export function ChatHeader({ title, onToggleSidebar, onClose, watching = [], sessionsOpen = false, onToggleSessions }: Props) {
  // Browser-only state, read after mount so the server-rendered markup matches.
  const [canAlert, setCanAlert] = useState(false)
  const [alertsOn, setAlertsOn] = useState(false)
  const [denied, setDenied] = useState(false)
  useEffect(() => {
    setCanAlert(alertsSupported())
    setAlertsOn(alertsEnabled())
  }, [])

  const toggleAlerts = async () => {
    const on = await setAlertsEnabled(!alertsOn)
    setDenied(!alertsOn && !on)
    setAlertsOn(on)
  }

  const alertTitle = denied
    ? 'Notifications are blocked for this site in the browser settings'
    : alertsOn
      ? 'Reply alerts on: you are notified when a reply finishes while HQ is in the background'
      : 'Notify me when a reply finishes while HQ is in the background'

  return (
    <div className="flex items-center gap-1 border-b border-white/10 px-1 md:px-3 shrink-0">
      <button type="button" onClick={onToggleSidebar} className="p-2.5 text-neutral-400 hover:text-white" title="Toggle chat list">
        <PanelLeft className="w-4 h-4" />
      </button>
      <span className="flex-1 text-sm text-neutral-300 truncate py-2.5">{title}</span>
      {watching.length > 0 && onToggleSessions && (
        <WatchingButton sessions={watching} open={sessionsOpen} onToggle={onToggleSessions} />
      )}
      {canAlert && (
        <button
          type="button"
          onClick={() => void toggleAlerts()}
          className={`p-2.5 rounded-lg hover:bg-white/10 transition-colors ${alertsOn ? 'text-emerald-400' : 'text-neutral-500 hover:text-white'}`}
          title={alertTitle}
          aria-pressed={alertsOn}
        >
          {alertsOn ? <Bell className="w-4 h-4" /> : <BellOff className="w-4 h-4" />}
        </button>
      )}
      {onClose && (
        <>
          <kbd className="hidden sm:inline text-[11px] text-neutral-400 bg-white/5 px-2 py-0.5 rounded border border-white/5">⌘K</kbd>
          <button
            type="button"
            onClick={onClose}
            className="p-2.5 rounded-lg hover:bg-white/10 text-neutral-400 hover:text-white transition-colors"
            title="Close chat"
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
      title="Coding-agent sessions this chat is watching"
      aria-label={watchingLabel(sessions)}
      aria-expanded={open}
    >
      <Eye className="w-4 h-4" />
      <span className="hidden sm:inline">Watching</span> {sessions.length}
      {waiting > 0 && (
        <span className="flex items-center gap-1" style={{ color: 'var(--accent-amber)' }} aria-hidden="true">
          <span className="w-1.5 h-1.5 rounded-full bg-current" />
          {waiting}
        </span>
      )}
    </button>
  )
}
