// The one route table: every web address, the phone link beside it, and the old addresses that
// still redirect to it (web wave 2 spec 01). Old addresses live forever: links to them sit in Inbox
// items, pushes, printed QR codes and the phone's own builds.

import { hashRedirect } from './settingsGroups'

export interface RouteRow {
  web: string
  /** The phone's `hq://` link, or null where the web alone has the address. Never carries a `?q=` (B6). */
  phone: string | null
  /** Old addresses that land here. */
  replaces: string[]
  /** A Settings screen only the phone has: the web never draws a row, and its address opens the Settings home. */
  phoneOnly?: boolean
}

export const ROUTE_TABLE: RouteRow[] = [
  { web: '/', phone: 'hq://chat', replaces: [] },
  { web: '/chat', phone: 'hq://chat', replaces: [] },
  { web: '/chat/:id', phone: 'hq://chat/{id}', replaces: ['/chat?thread={id}'] },
  { web: '/chat?compose=1', phone: 'hq://chat?compose=1', replaces: [] },
  { web: '/notes', phone: 'hq://notes', replaces: ['/vault'] },
  { web: '/notes/$path', phone: 'hq://notes/{path}', replaces: ['/notes/$path'] },
  { web: '/notes/n/:id', phone: 'hq://notes/n/{id}', replaces: [] },
  { web: '/tasks', phone: 'hq://tasks', replaces: [] },
  { web: '/tasks/:id', phone: 'hq://tasks/{id}', replaces: ['/tasks?task={id}'] },
  { web: '/tasks/away', phone: 'hq://tasks/away', replaces: [] },
  { web: '/tasks?view=board', phone: 'hq://web/board', replaces: [] },
  { web: '/tasks?view=timeline', phone: 'hq://web/timeline', replaces: [] },
  { web: '/helpers', phone: 'hq://helpers', replaces: ['/sessions'] },
  { web: '/helpers/:id', phone: 'hq://helpers/{id}', replaces: ['/sessions?id={id}'] },
  { web: '/inbox', phone: 'hq://inbox', replaces: ['/notifications'] },
  { web: '/inbox/:id', phone: 'hq://inbox/{id}', replaces: [] },
  { web: '/settings/devices', phone: 'hq://settings/devices', replaces: ['/settings#devices'] },
  { web: '/settings/browser', phone: null, replaces: [] },
  { web: '/settings/spending', phone: 'hq://web/usage', replaces: ['/usage'] },
  { web: '/settings/assistant', phone: 'hq://settings/assistant', replaces: [], phoneOnly: true },
  { web: '/settings/update', phone: 'hq://settings/update', replaces: [], phoneOnly: true },
  { web: '/settings/speech', phone: 'hq://settings/speech', replaces: [], phoneOnly: true },
  { web: '/setup', phone: null, replaces: [] },
  { web: '/connect', phone: null, replaces: [] },
  { web: '/signin', phone: null, replaces: [] },
  { web: '?q=', phone: 'hq://search', replaces: [] },
]

export type Edition = 'full' | 'lite' | 'mobile'

export interface Place {
  id: 'notes' | 'tasks' | 'chat' | 'helpers' | 'inbox'
  path: '/notes' | '/tasks' | '/chat' | '/helpers' | '/inbox'
  /** The phone's page (`hq://{place}`) when an embedded page leaves for another place. */
  phone: string
}

/**
 * Every place that has an address. Only three are on the rail and the bar (`BAR_PLACES`): Tasks and
 * Helpers are where HQ and its agents work, so a person reaches them from an Inbox item, a chat
 * card or Settings, never from the bar.
 */
export const PLACES: Place[] = [
  { id: 'notes', path: '/notes', phone: 'hq://notes' },
  { id: 'tasks', path: '/tasks', phone: 'hq://tasks' },
  { id: 'chat', path: '/chat', phone: 'hq://chat' },
  { id: 'helpers', path: '/helpers', phone: 'hq://helpers' },
  { id: 'inbox', path: '/inbox', phone: 'hq://inbox' },
]

/** The rail and bar order: Notes, Chat, Inbox. */
export const BAR_PLACES: Place['id'][] = ['notes', 'chat', 'inbox']

/** The Settings pages that have an address (a name beyond these opens the Settings home). */
export const SETTINGS_SECTIONS = [
  'browser',
  'time',
  'version',
  'data',
  'devices',
  'github',
  'keys',
  'messaging',
  'safety',
  'folders',
  'backup',
  'spending',
] as const

/** Where a link goes: a path, a search object and a fragment, so a router can take it as it is. */
export interface Target {
  to: string
  search: Record<string, string>
  hash: string
}

export interface Arrival {
  pathname: string
  /** The raw query string with or without its `?`. */
  search: string
  /** The raw fragment with or without its `#`. */
  hash: string
}

export interface RedirectContext {
  edition?: Edition | null
  /** The page was opened by a login link or a QR code: after it signs in, HQ opens Chat. */
  signInLink?: boolean
}

const trimPath = (p: string) => (p.length > 1 ? p.replace(/\/+$/, '') : p)
const joinId = (base: string, id: string) => `${base}/${encodeURIComponent(id)}`

/** The first place a person lands. Lite has no Chat, so it opens Notes. */
export const startPlace = (edition?: Edition | null): '/chat' | '/notes' => (edition === 'lite' ? '/notes' : '/chat')

/** The `?a=b` and `#c` of a target, for a plain href. */
export function targetHref(t: Target): string {
  const query = new URLSearchParams(t.search).toString()
  return `${t.to}${query ? `?${query}` : ''}${t.hash ? `#${t.hash.replace(/^#/, '')}` : ''}`
}

/**
 * Where an old address goes now, or null when the address is current. The item and the fragment
 * stay: `/sessions?id=h-31#x` is `/helpers/h-31#x`. Sign-in fragments are read before any router
 * runs (`adoptLinkToken`), so by here they are gone and `signInLink` says one was there.
 */
export function legacyRedirect(from: Arrival, ctx: RedirectContext = {}): Target | null {
  const path = trimPath(from.pathname)
  const params = new URLSearchParams(from.search.replace(/^\?/, ''))
  const hash = from.hash.replace(/^#/, '')
  const keep = (to: string, drop: string[] = []): Target => {
    const search: Record<string, string> = {}
    params.forEach((v, k) => {
      if (!drop.includes(k)) search[k] = v
    })
    return { to, search, hash }
  }

  if (path === '/') return keep(startPlace(ctx.edition))

  if (path === '/vault') return keep(ctx.signInLink ? startPlace(ctx.edition) : '/notes')
  if (path.startsWith('/vault/')) return keep(`/notes${path.slice('/vault'.length)}`)

  if (path === '/chat') {
    const thread = params.get('thread')
    return thread ? keep(joinId('/chat', thread), ['thread']) : null
  }

  if (path === '/tasks') {
    const task = params.get('task')
    return task ? keep(joinId('/tasks', task), ['task']) : null
  }

  if (path === '/sessions' || path.startsWith('/sessions/')) {
    const id = params.get('id')
    return id ? keep(joinId('/helpers', id), ['id']) : keep('/helpers')
  }

  if (path === '/notifications' || path.startsWith('/notifications/')) return keep(`/inbox${path.slice('/notifications'.length)}`)

  if (path === '/usage') return keep('/settings/spending')

  if (path === '/settings') {
    const to = hashRedirect(hash)
    return to ? { to, search: {}, hash: '' } : null
  }

  if (path.startsWith('/settings/')) {
    const section = path.slice('/settings/'.length)
    const known = (SETTINGS_SECTIONS as readonly string[]).includes(section)
    return known ? null : { to: '/settings', search: {}, hash: '' }
  }

  return null
}

export interface LiteRedirect {
  target: Target
  /** The sentence to show once, as a catalogue key plus the place it names. */
  notice: { key: 'lite_no_place'; place: 'chat' | 'helpers' } | { key: 'lite_no_spending' } | null
}

const LITE_SETTINGS = ['browser', 'time', 'version', 'data', 'devices']

/** Lite has no Chat, Helpers or Spending: a link to one opens Notes, or This browser, with a line saying so (WR4, WR28, B13). */
export function liteRedirect(pathname: string): LiteRedirect | null {
  const path = trimPath(pathname)
  const to = (p: string): Target => ({ to: p, search: {}, hash: '' })
  if (path === '/chat' || path.startsWith('/chat/')) return { target: to('/notes'), notice: { key: 'lite_no_place', place: 'chat' } }
  if (path === '/helpers' || path.startsWith('/helpers/') || path === '/sessions' || path.startsWith('/sessions/')) {
    return { target: to('/notes'), notice: { key: 'lite_no_place', place: 'helpers' } }
  }
  if (path === '/settings/spending') return { target: to('/settings/browser'), notice: { key: 'lite_no_spending' } }
  if (path.startsWith('/settings/') && !LITE_SETTINGS.includes(path.slice('/settings/'.length))) {
    return { target: to('/settings/browser'), notice: null }
  }
  return null
}

/** The place an address belongs to, so the rail stays lit on every address under it (Drift 12). */
export function placeOf(pathname: string): Place['id'] | null {
  const first = pathname.split('/')[1] ?? ''
  return PLACES.find((p) => p.id === first)?.id ?? null
}

/** The page that a place's address opens inside the phone: `hq://{place}` with the item after it. */
export function phoneLink(pathname: string): string | null {
  const place = PLACES.find((p) => p.id === placeOf(pathname))
  if (!place) return null
  const rest = pathname.split('/').slice(2).join('/')
  return rest ? `${place.phone}/${rest}` : place.phone
}
