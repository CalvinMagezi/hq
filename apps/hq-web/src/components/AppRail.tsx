import { Link } from '@tanstack/react-router'
import { Bell, BookOpen, CheckSquare, Gauge, MessageSquare, SlidersHorizontal, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { needsYouAria } from '~/lib/workbench'

interface RailLinkProps {
  to: '/vault' | '/chat' | '/tasks' | '/sessions' | '/usage' | '/notifications' | '/settings'
  label: string
  exact?: boolean
  children: React.ReactNode
}

function RailLink({ to, label, exact, children }: RailLinkProps) {
  return (
    <Link to={to} search={{}} title={label} aria-label={label} className="hq-rail-item" activeOptions={{ exact: exact ?? false, includeSearch: false }}>
      {children}
    </Link>
  )
}

/** The desktop navigation: one column of icons on the left edge. Phones use the BottomNav instead. */
export function AppRail() {
  const unread = inboxBadge(useHQStore((s) => s.unreadNotificationsCount))
  const needsYouCount = useHQStore((s) => s.needsYouCount)
  const needsYou = inboxBadge(needsYouCount)
  return (
    <nav className="hq-rail" aria-label="Main">
      <Link to="/vault" search={{}} aria-label="Agent HQ home" className="mb-3 mt-1 transition-opacity active:opacity-60">
        <img src="/icons/hq-mark-96.png" alt="" width="32" height="32" className="object-contain" />
      </Link>
      <RailLink to="/vault" label="Home" exact>
        <BookOpen className="w-5 h-5" />
      </RailLink>
      <RailLink to="/chat" label="Chat" exact>
        <MessageSquare className="w-5 h-5" />
      </RailLink>
      <RailLink to="/tasks" label="Tasks" exact>
        <CheckSquare className="w-5 h-5" />
      </RailLink>
      <RailLink to="/sessions" label={needsYou ? `Workbench, ${needsYouAria(needsYouCount)}` : 'Workbench'}>
        <Terminal className="w-5 h-5" />
        {needsYou && <span className="hq-rail-badge">{needsYou.label}</span>}
      </RailLink>
      <RailLink to="/usage" label="Usage">
        <Gauge className="w-5 h-5" />
      </RailLink>
      <RailLink to="/notifications" label={unread ? `Inbox, ${unread.aria}` : 'Inbox'} exact>
        <Bell className="w-5 h-5" />
        {unread && <span className="hq-rail-badge">{unread.label}</span>}
      </RailLink>
      <div className="flex-1" />
      <RailLink to="/settings" label="Settings">
        <SlidersHorizontal className="w-5 h-5" />
      </RailLink>
    </nav>
  )
}
