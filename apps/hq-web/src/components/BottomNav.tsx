import { t } from '~/strings'
import { Link } from '@tanstack/react-router'
import { Bell, BookOpen, CheckSquare, MessageSquare, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { needsYouAria } from '~/lib/workbench'
import { placesFor, useEdition } from '~/lib/edition'
import type { Place } from '~/lib/routes'
import { inboxNeedsYou } from './AppRail'

const itemClass = 'flex-1 min-w-0 flex flex-col items-center justify-center gap-1 text-center transition-colors'

const ICONS: Record<Place['id'], typeof BookOpen> = {
  notes: BookOpen,
  tasks: CheckSquare,
  chat: MessageSquare,
  helpers: Terminal,
  inbox: Bell,
}

const LABELS: Record<Place['id'], 'place_notes' | 'place_tasks' | 'place_chat' | 'place_helpers' | 'place_inbox'> = {
  notes: 'place_notes',
  tasks: 'place_tasks',
  chat: 'place_chat',
  helpers: 'place_helpers',
  inbox: 'place_inbox',
}

export function BottomNav() {
  const helpers = useHQStore((s) => s.needsYouCount)
  const pending = useHQStore((s) => s.pendingApprovalsCount)
  const needsYou = inboxBadge(inboxNeedsYou(pending, helpers))
  const places = placesFor(useEdition())
  return (
    <nav className="hq-bottom-nav">
      {places.map((place) => {
        const Icon = ICONS[place.id]
        const badge = place.id === 'inbox' ? needsYou : null
        return (
          <Link
            key={place.id}
            to={place.path}
            search={{}}
            className={itemClass}
            activeProps={{ style: { color: 'var(--ink)' } }}
            inactiveProps={{ style: { color: 'var(--ink-muted)' } }}
            activeOptions={{ exact: false, includeSearch: false }}
          >
            <div className="relative flex items-center justify-center">
              <Icon width={17} height={17} strokeWidth={2} />
              {badge && (
                <span
                  role="status"
                  aria-label={needsYouAria(inboxNeedsYou(pending, helpers))}
                  className="absolute -top-1.5 left-2.5 min-w-[16px] h-4 px-1 rounded-full text-xs leading-4 text-center font-bold text-black pointer-events-none"
                  style={{ background: 'var(--warn)' }}
                >
                  {badge.label}
                </span>
              )}
            </div>
            <span className="text-xs font-semibold tracking-wide">{t(LABELS[place.id])}</span>
          </Link>
        )
      })}
    </nav>
  )
}
