export interface KeyLike {
  ctrlKey: boolean
  altKey: boolean
  metaKey: boolean
  shiftKey: boolean
  code: string
  key: string
  target?: EventTarget | null
}

const TEXT_TAGS = ['INPUT', 'TEXTAREA', 'SELECT']

/** Ctrl Alt N opens a new note; plain Ctrl N is the browser's new window, so it is left alone. */
export function opensQuickNote(e: KeyLike): boolean {
  if (!e.ctrlKey || !e.altKey || e.metaKey || e.shiftKey || e.code !== 'KeyN') return false
  const el = e.target as { tagName?: string; isContentEditable?: boolean } | null | undefined
  const tag = el?.tagName?.toUpperCase()
  return !(tag && TEXT_TAGS.includes(tag)) && !el?.isContentEditable
}

export const savesQuickNote = (e: Pick<KeyLike, 'ctrlKey' | 'metaKey' | 'key'>): boolean =>
  (e.ctrlKey || e.metaKey) && e.key === 'Enter'
