// The work pane beside a chat: which tab shows, what it points at, and when it opens by itself.
// Decided with the owner: it opens by itself once per chat, when HQ makes something, and stays
// closed after the person closes it.

export type PaneTab = 'result' | 'context' | 'helpers'

export interface PaneState {
  open: boolean
  tab: PaneTab
  /** The note the Result tab shows. */
  note: string | null
  /** The source the Context tab has selected, by its path or address. */
  source: string | null
}

export const CLOSED_PANE: PaneState = { open: false, tab: 'result', note: null, source: null }

const SEEN_KEY = 'hq-pane-auto'

/** Chats whose pane has already opened by itself. A private window or blocked storage reads as none. */
export function readAutoOpened(): string[] {
  try {
    const raw = window.localStorage.getItem(SEEN_KEY)
    const list: unknown = raw ? JSON.parse(raw) : []
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

export function writeAutoOpened(ids: string[]): void {
  try {
    // A long-lived install would otherwise keep every chat id it ever opened.
    window.localStorage.setItem(SEEN_KEY, JSON.stringify(ids.slice(-200)))
  } catch {
    // Without storage the pane may open once more after a reload, which is harmless.
  }
}

interface AutoOpenInput {
  /** A note HQ made in this chat that the pane could show, or null. */
  made: string | null
  /** The pane has room beside the chat; below that the card in the thread is the door. */
  docked: boolean
  /** This chat's pane has opened by itself before. */
  already: boolean
}

/** Whether the pane should open now by itself: once per chat, only with something to show and room for it. */
export function shouldAutoOpen({ made, docked, already }: AutoOpenInput): boolean {
  return made !== null && docked && !already
}

/** A Vault note's folder as the card says it: "Vault › Daily" for "Notebooks/Daily/Plan.md". */
export function folderLabel(path: string): string {
  const parts = path.split('/').filter(Boolean)
  const folders = parts.slice(1, -1)
  return ['Vault', ...folders].join(' › ')
}
