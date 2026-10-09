import { Link } from '@tanstack/react-router'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { needsYouAria } from '~/lib/workbench'

const itemClass = 'flex-1 min-w-0 flex flex-col items-center justify-center gap-1 text-center transition-colors'

export function BottomNav() {
  const badge = inboxBadge(useHQStore((s) => s.unreadNotificationsCount))
  const needsYouCount = useHQStore((s) => s.needsYouCount)
  const needsYouBadge = inboxBadge(needsYouCount)
  const hasPending = useHQStore((s) => s.pendingApprovalsCount > 0)
  return (
    <nav
      className="hq-bottom-nav"
    >
      <Link
        to="/vault"
        search={{}}
        className={itemClass}
        activeProps={{ style: { color: 'var(--accent-green)' } }}
        inactiveProps={{ style: { color: 'var(--text-dim)' } }}
        activeOptions={{ exact: true, includeSearch: false }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
          <polyline points="9 22 9 12 15 12 15 22" />
        </svg>
        <span className="text-[11px] font-semibold tracking-wide">Home</span>
      </Link>

      <Link
        to="/chat"
        className={itemClass}
        activeProps={{ style: { color: 'var(--accent-green)' } }}
        inactiveProps={{ style: { color: 'var(--text-dim)' } }}
        activeOptions={{ exact: true }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
        <span className="text-[11px] font-semibold tracking-wide">Chat</span>
      </Link>

      <Link
        to="/tasks"
        className={itemClass}
        activeProps={{ style: { color: 'var(--accent-green)' } }}
        inactiveProps={{ style: { color: 'var(--text-dim)' } }}
        activeOptions={{ exact: true }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="m9 12 2 2 4-4" />
        </svg>
        <span className="text-[11px] font-semibold tracking-wide">Tasks</span>
      </Link>

      <Link
        to="/sessions"
        className={itemClass}
        activeProps={{ style: { color: 'var(--accent-green)' } }}
        inactiveProps={{ style: { color: 'var(--text-dim)' } }}
      >
        <div className="relative flex items-center justify-center">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
          {needsYouBadge && (
            <span
              role="status"
              aria-label={needsYouAria(needsYouCount)}
              className="absolute -top-1.5 left-2.5 min-w-[16px] h-4 px-1 rounded-full text-[9px] leading-4 text-center font-bold text-black pointer-events-none"
              style={{ background: 'var(--accent-green)' }}
            >
              {needsYouBadge.label}
            </span>
          )}
        </div>
        <span className="text-[11px] font-semibold tracking-wide">Workbench</span>
      </Link>

      <Link
        to="/notifications"
        className={itemClass}
        activeProps={{ style: { color: 'var(--accent-green)' } }}
        inactiveProps={{ style: { color: 'var(--text-dim)' } }}
        activeOptions={{ exact: true }}
      >
        <div className="relative flex items-center justify-center">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
            <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
          </svg>
          {badge && (
            <span
              role="status"
              aria-label={badge.aria}
              className={`absolute -top-1.5 left-2.5 min-w-[16px] h-4 px-1 rounded-full text-[9px] leading-4 text-center font-bold pointer-events-none ${
                hasPending ? 'text-black' : 'text-white bg-white/20'
              }`}
              style={hasPending ? { background: 'var(--accent-green)' } : undefined}
            >
              {badge.label}
            </span>
          )}
        </div>
        <span className="text-[11px] font-semibold tracking-wide">Inbox</span>
      </Link>
    </nav>
  )
}

