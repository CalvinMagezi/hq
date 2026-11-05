import { t } from '~/strings'
import { Link } from '@tanstack/react-router'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { needsYouAria } from '~/lib/workbench'
import { inboxNeedsYou } from './AppRail'

const itemClass = 'flex-1 min-w-0 flex flex-col items-center justify-center gap-1 text-center transition-colors'

export function BottomNav() {
  const helpers = useHQStore((s) => s.needsYouCount)
  const pending = useHQStore((s) => s.pendingApprovalsCount)
  const needsYou = inboxBadge(inboxNeedsYou(pending, helpers))
  return (
    <nav
      className="hq-bottom-nav"
    >
      <Link
        to="/vault"
        search={{}}
        className={itemClass}
        activeProps={{ style: { color: 'var(--ink)' } }}
        inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
        activeOptions={{ exact: true, includeSearch: false }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
          <polyline points="9 22 9 12 15 12 15 22" />
        </svg>
        <span className="text-xs font-semibold tracking-wide">Home</span>
      </Link>

      <Link
        to="/chat"
        className={itemClass}
        activeProps={{ style: { color: 'var(--ink)' } }}
        inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
        activeOptions={{ exact: true }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
        <span className="text-xs font-semibold tracking-wide">Chat</span>
      </Link>

      <Link
        to="/tasks"
        className={itemClass}
        activeProps={{ style: { color: 'var(--ink)' } }}
        inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
        activeOptions={{ exact: true }}
      >
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="m9 12 2 2 4-4" />
        </svg>
        <span className="text-xs font-semibold tracking-wide">Tasks</span>
      </Link>

      <Link
        to="/sessions"
        className={itemClass}
        activeProps={{ style: { color: 'var(--ink)' } }}
        inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
      >
        <div className="relative flex items-center justify-center">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
        </div>
        <span className="text-xs font-semibold tracking-wide">{t('place_helpers')}</span>
      </Link>

      <Link
        to="/notifications"
        className={itemClass}
        activeProps={{ style: { color: 'var(--ink)' } }}
        inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
        activeOptions={{ exact: true }}
      >
        <div className="relative flex items-center justify-center">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
            <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
          </svg>
          {needsYou && (
            <span
              role="status"
              aria-label={needsYouAria(inboxNeedsYou(pending, helpers))}
              className="absolute -top-1.5 left-2.5 min-w-[16px] h-4 px-1 rounded-full text-xs leading-4 text-center font-bold text-black pointer-events-none"
              style={{ background: 'var(--warn)' }}
            >
              {needsYou.label}
            </span>
          )}
        </div>
        <span className="text-xs font-semibold tracking-wide">Inbox</span>
      </Link>
    </nav>
  )
}

