import { t } from '~/strings'
import { Link } from '@tanstack/react-router'
import { Bell, BookOpen, CheckSquare, MessageSquare, SlidersHorizontal, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { needsYouAria } from '~/lib/workbench'
import { placesFor, useEdition } from '~/lib/edition'
import type { Place } from '~/lib/routes'

/** One count for the Inbox badge: what needs the owner, helpers included (W6). */
export const inboxNeedsYou = (pending: number, helpers: number) => pending + helpers

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

/** The desktop navigation: one column of icons on the left edge. Phones use the BottomNav instead. */
export function AppRail() {
  const helpers = useHQStore((s) => s.needsYouCount)
  const pending = useHQStore((s) => s.pendingApprovalsCount)
  const needsYou = inboxBadge(inboxNeedsYou(pending, helpers))
  const places = placesFor(useEdition())
  return (
    <nav className="hq-rail" aria-label="Main">
      <Link to="/" search={{}} aria-label="Agent HQ home" className="mb-3 mt-1 transition-opacity active:opacity-60">
        <img src="/icons/hq-mark-96.png" alt="" width="32" height="32" className="object-contain" />
      </Link>
      {places.map((place) => {
        const Icon = ICONS[place.id]
        const badge = place.id === 'inbox' ? needsYou : null
        const label = badge ? `${t(LABELS[place.id])}, ${needsYouAria(inboxNeedsYou(pending, helpers))}` : t(LABELS[place.id])
        return (
          // Not exact: the place stays lit on every address under it (/notes/Projects/Plan.md lights Notes).
          <Link key={place.id} to={place.path} search={{}} title={label} aria-label={label} className="hq-rail-item" activeOptions={{ exact: false, includeSearch: false }}>
            <Icon className="w-5 h-5" />
            {badge && <span className="hq-rail-badge">{badge.label}</span>}
          </Link>
        )
      })}
      <div className="flex-1" />
      <Link to="/settings" search={{}} title={t('place_settings')} aria-label={t('place_settings')} className="hq-rail-item" activeOptions={{ exact: false, includeSearch: false }}>
        <SlidersHorizontal className="w-5 h-5" />
      </Link>
    </nav>
  )
}
