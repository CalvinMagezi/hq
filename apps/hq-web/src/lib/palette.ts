// The palette's own rows: the actions it can do, folders that match what was typed, and the lines of a
// note it shows before opening it. The rest of the palette is the search in `lib/search.ts`.
import type { NoteTreeNode } from './vaultApi'
import { matchWords } from './search'

export type DoId = 'new-note' | 'new-chat' | 'pane' | 'go-vault' | 'go-chat' | 'go-inbox' | 'go-settings'

export interface DoAction {
  id: DoId
  /** The catalogue key of its name. */
  label: 'palette_new_note' | 'palette_new_chat' | 'palette_pane' | 'palette_go_vault' | 'palette_go_chat' | 'palette_go_inbox' | 'palette_go_settings'
  /** The chord that does it without the palette; every action shows its key, so nobody needs a tutorial. */
  keys?: string
}

const ALL: DoAction[] = [
  { id: 'new-note', label: 'palette_new_note', keys: 'Ctrl Alt N' },
  { id: 'new-chat', label: 'palette_new_chat', keys: 'Ctrl Shift O' },
  { id: 'pane', label: 'palette_pane', keys: 'Ctrl .' },
  { id: 'go-vault', label: 'palette_go_vault' },
  { id: 'go-chat', label: 'palette_go_chat' },
  { id: 'go-inbox', label: 'palette_go_inbox' },
  { id: 'go-settings', label: 'palette_go_settings' },
]

interface Where {
  /** A chat is open, so its work pane can be toggled. */
  inChat: boolean
  /** HQ Lite runs no chats. */
  lite: boolean
}

/** The actions that make sense here: the pane only in a chat, and no chat actions on Lite. */
export function doActions({ inChat, lite }: Where): DoAction[] {
  return ALL.filter((a) => {
    if (lite && (a.id === 'new-chat' || a.id === 'pane' || a.id === 'go-chat')) return false
    return a.id !== 'pane' || inChat
  })
}

/** The actions whose name holds every word typed. */
export function matchActions(actions: DoAction[], query: string, name: (a: DoAction) => string): DoAction[] {
  return actions.filter((a) => matchWords(query, name(a)))
}

export interface FolderHit {
  path: string
  /** "Shop › Orders": the path under the Vault's top folder. */
  trail: string
}

/** Folders whose path holds every word typed ("shop orders" finds Projects/Shop/Orders). System folders are left out. */
export function matchFolders(root: NoteTreeNode | undefined, query: string, limit = 5): FolderHit[] {
  if (!root || !query.trim()) return []
  const out: FolderHit[] = []
  const walk = (node: NoteTreeNode) => {
    for (const child of node.children ?? []) {
      if (child.type !== 'dir' || child.name.startsWith('_')) continue
      const under = child.path.split('/').slice(1)
      if (matchWords(query, under.join(' '))) out.push({ path: child.path, trail: under.join(' › ') })
      walk(child)
    }
  }
  walk(root)
  // The shortest path first: a folder named for the words beats one that merely lies under another.
  return out.sort((a, b) => a.path.length - b.path.length || a.path.localeCompare(b.path)).slice(0, limit)
}

/** The first lines of a note for the preview: no frontmatter, no blank lines, no more than `max`. */
export function previewLines(content: string, max = 8): string[] {
  const body = content.replace(/^---\n[\s\S]*?\n---\n?/, '')
  return body
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .slice(0, max)
}

/** A person who asked the browser to save data gets no preview fetched in the background. */
export function previewAllowed(connection: { saveData?: boolean } | undefined): boolean {
  return connection?.saveData !== true
}
