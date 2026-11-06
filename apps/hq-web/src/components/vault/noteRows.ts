import type { NoteTreeNode } from '~/lib/vaultApi'

const NOTEBOOKS_ROOT = 'Notebooks'
const TRAIL_SEPARATOR = ' › '
/** A folder lists this many notes before "See all". */
export const FOLDER_NOTE_LIMIT = 25

/** The folders a note sits in, the way the phone shows them: Notebooks is the root and is left out. */
export function folderTrail(path: string): string {
  const folders = path.split('/').slice(0, -1)
  if (folders[0] === NOTEBOOKS_ROOT) folders.shift()
  return folders.join(TRAIL_SEPARATOR)
}

/** The name a person reads for a file: no folder and no `.md`. */
export function noteName(path: string): string {
  return (path.split('/').pop() ?? path).replace(/\.md$/, '')
}

/** The folder a path sits in, or null for a top-level path. */
export function parentPath(path: string): string | null {
  const at = path.lastIndexOf('/')
  return at > 0 ? path.slice(0, at) : null
}

/** One breadcrumb per folder above the path's last part, each with the path it opens. */
export function crumbsOf(path: string): { name: string; path: string }[] {
  const parts = path.split('/').slice(0, -1)
  return parts
    .map((name, i) => ({ name, path: parts.slice(0, i + 1).join('/') }))
    .filter((c) => c.name !== NOTEBOOKS_ROOT)
}

/** Folders worth a chip on home: the top folders under Notebooks, without the hidden `_` ones. */
export function topFolders(tree: NoteTreeNode | null | undefined): NoteTreeNode[] {
  return (tree?.children ?? []).filter((c) => c.type === 'dir' && !c.name.startsWith('_'))
}

/** Notes inside a folder at any depth. */
export function countNotes(node: NoteTreeNode): number {
  if (node.type === 'file') return node.name.endsWith('.md') ? 1 : 0
  return (node.children ?? []).reduce((sum, c) => sum + countNotes(c), 0)
}

/** Every folder for the Save to list, parents before children, each with how deep it sits under Notebooks. */
export function folderChoices(folders: readonly string[]): { path: string; name: string; depth: number }[] {
  return [...folders]
    .filter((f) => f !== NOTEBOOKS_ROOT)
    .sort((a, b) => a.localeCompare(b))
    .map((path) => {
      const parts = path.split('/')
      const under = parts[0] === NOTEBOOKS_ROOT ? 1 : 0
      return { path, name: parts[parts.length - 1], depth: parts.length - 1 - under }
    })
}
