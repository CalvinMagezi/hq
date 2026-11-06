import { t } from '~/strings'
import { Link } from '@tanstack/react-router'
import { Bell, BookOpen, CheckSquare, Settings, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { inboxBadge } from '~/lib/inboxBadge'
import { placesFor, useEdition } from '~/lib/edition'
import type { Place } from '~/lib/routes'

/** One count for the Inbox badge: what needs the owner, helpers included (W6). */
export const inboxNeedsYou = (pending: number, helpers: number) => pending + helpers

const ICONS: Record<Exclude<Place['id'], 'chat'>, typeof BookOpen> = {
  notes: BookOpen,
  tasks: CheckSquare,
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

/** The Inbox's spoken name with its count, the way the design reads it: "Inbox, 1 needs you". */
export function inboxAria(count: number): string {
  return count === 1 ? t('nav_inbox_needs_one') : t('nav_inbox_needs_many', { n: count })
}

/** The five places, as the rail (`a-rail-item`) or the phone's bottom bar (`a-nav-item`) draws them. Chat is raised on the HQ mark. */
export function PlaceLinks({ kind }: { kind: 'rail' | 'nav' }) {
  const helpers = useHQStore((s) => s.needsYouCount)
  const pending = useHQStore((s) => s.pendingApprovalsCount)
  const count = inboxNeedsYou(pending, helpers)
  const badge = inboxBadge(count)
  const places = placesFor(useEdition())
  const item = kind === 'rail' ? 'a-rail-item' : 'a-nav-item'
  return (
    <>
      {places.map((place) => {
        const label = t(LABELS[place.id])
        const chat = place.id === 'chat'
        const withBadge = place.id === 'inbox' && badge
        const Icon = place.id === 'chat' ? null : ICONS[place.id]
        return (
          // Not exact: the place stays lit on every address under it (/notes/Projects/Plan.md lights Notes).
          <Link
            key={place.id}
            to={place.path}
            search={{}}
            className={chat ? `${item} ${kind === 'rail' ? 'a-rail-chat' : 'a-nav-chat'}` : item}
            aria-label={withBadge ? `${label}, ${inboxAria(count)}` : undefined}
            activeOptions={{ exact: false, includeSearch: false }}
          >
            {chat ? (
              <span className="a-hqb">
                <span className="a-mark" />
              </span>
            ) : (
              <span className="a-pill">
                {Icon && <Icon className="a-ic" aria-hidden="true" />}
                {withBadge && <span className="a-badge">{badge.label}</span>}
              </span>
            )}
            {label}
          </Link>
        )
      })}
    </>
  )
}

/** Settings at the foot of the rail; on a phone it is the gear in the top bar. */
export function SettingsRailLink() {
  return (
    <Link to="/settings" search={{}} className="a-rail-item" activeOptions={{ exact: false, includeSearch: false }}>
      <span className="a-pill">
        <Settings className="a-ic" aria-hidden="true" />
      </span>
      {t('place_settings')}
    </Link>
  )
}
