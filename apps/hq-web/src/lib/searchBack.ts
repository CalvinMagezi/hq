import type { SearchScope } from './search'

/** What Back brings back: the words, scope, folder and scroll of the Search a result was opened from. */
export interface SearchBack {
  q: string
  scope: SearchScope
  folder: string | null
  scroll: number
}

const KEY = 'hqSearch'
const SCOPE_NAMES: SearchScope[] = ['all', 'notes', 'tasks', 'chats', 'helpers']

/** The address of the entry Search was opened over, carrying `?q=` so Back and Forward can find it again. */
export function withQuery(href: string, q: string): string {
  const url = new URL(href, 'http://x.invalid')
  url.searchParams.set('q', q)
  return url.pathname + url.search + url.hash
}

export function withoutQuery(href: string): string {
  const url = new URL(href, 'http://x.invalid')
  url.searchParams.delete('q')
  return url.pathname + url.search + url.hash
}

/** The Search to reopen for a history entry, or null when the entry was not left from Search. */
export function readBack(search: string, state: unknown): SearchBack | null {
  const q = new URLSearchParams(search).get('q')
  const saved = (state as Record<string, unknown> | null)?.[KEY] as Partial<SearchBack> | undefined
  if (!q || !q.trim() || !saved || saved.q !== q) return null
  const scope = SCOPE_NAMES.includes(saved.scope as SearchScope) ? (saved.scope as SearchScope) : 'all'
  return { q, scope, folder: typeof saved.folder === 'string' ? saved.folder : null, scroll: Number(saved.scroll) || 0 }
}

/** Marks the current history entry so the Back that returns to it reopens Search once. */
export function rememberSearch(back: SearchBack, win: Pick<Window, 'history' | 'location'> = window): void {
  const { pathname, search, hash } = win.location
  const next = withQuery(pathname + search + hash, back.q)
  win.history.replaceState({ ...(win.history.state ?? {}), [KEY]: back }, '', next)
}

/** Drops the marker once Search has reopened, so a second Back goes where Search was opened from. */
export function forgetSearch(win: Pick<Window, 'history' | 'location'> = window): void {
  const { pathname, search, hash } = win.location
  const { [KEY]: _gone, ...rest } = (win.history.state ?? {}) as Record<string, unknown>
  win.history.replaceState(rest, '', withoutQuery(pathname + search + hash))
}
