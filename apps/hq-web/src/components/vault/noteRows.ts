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

// ---------------------------------------------------------------------------------------------
// Vault navigation: the location bar's trail and the folder tree's rows (board 5 to 7 of the
// "Restructure" canvas). Everything here is pure so the rules can be tested without a DOM.
// ---------------------------------------------------------------------------------------------

/** The name the person reads for the root of the notes, which is the `Notebooks` folder on disk. */
export const VAULT_NAME = 'Vault'

/** Every folder above a path, shallowest first: `a/b/c.md` gives `a` and `a/b`. */
export function ancestorPaths(path: string): string[] {
  const parts = path.split('/').slice(0, -1)
  return parts.map((_, i) => parts.slice(0, i + 1).join('/'))
}

export interface TrailPart {
  name: string
  /** The path it opens; empty for Vault home. */
  path: string
}

/**
 * The location bar's parts: Vault, each folder, then the open note. A folder is the last part
 * itself, so pass `isFile: false` for it. The `Notebooks` root is left out, as the folder page
 * always did.
 */
export function trailOf(path: string, isFile: boolean): TrailPart[] {
  const home: TrailPart = { name: VAULT_NAME, path: '' }
  if (!path) return [home]
  const folders = isFile ? crumbsOf(path) : crumbsOf(`${path}/x`)
  const trail = [home, ...folders]
  if (isFile) trail.push({ name: noteName(path), path })
  return trail
}

/** Whether a path names a file, when the tree is not loaded yet to say: it ends in an extension. */
export function looksLikeFile(path: string): boolean {
  const last = path.split('/').pop() ?? ''
  return /\.[A-Za-z0-9]{1,8}$/.test(last)
}

/** Where Up goes from a path: its folder, or Vault home (an empty path) from a top-level folder. Null on home. */
export function upTarget(path: string): string | null {
  if (!path) return null
  const parent = parentPath(path)
  return parent === null || parent === NOTEBOOKS_ROOT ? '' : parent
}

export interface FoldedTrail<T> {
  before: T[]
  hidden: T[]
  after: T[]
}

/**
 * Folds a long trail: the first part and the last `keepTail` always show, and the middle goes into
 * one "..." menu. A trail of `max` parts or fewer is left whole.
 */
export function foldTrail<T>(trail: readonly T[], max = 4, keepTail = 2): FoldedTrail<T> {
  if (trail.length <= max) return { before: [...trail], hidden: [], after: [] }
  return { before: trail.slice(0, 1), hidden: trail.slice(1, trail.length - keepTail), after: trail.slice(trail.length - keepTail) }
}

export type TreeRow =
  | { kind: 'folder'; path: string; name: string; depth: number; open: boolean; count: number; node: NoteTreeNode; system: boolean }
  | { kind: 'note'; path: string; name: string; depth: number }
  | { kind: 'more'; path: string; folder: string; name: string; depth: number; hiddenCount: number; total: number }

const byName = (a: NoteTreeNode, b: NoteTreeNode) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })

/** A folder the Vault keeps out of sight: HQ's own, which start with an underscore. */
export const isHiddenFolder = (node: NoteTreeNode) => node.type === 'dir' && node.name.startsWith('_')

const countCache = new WeakMap<NoteTreeNode, number>()
/** Notes inside a folder at any depth, remembered per node so a big tree is walked once. */
export function countNotesCached(node: NoteTreeNode): number {
  const known = countCache.get(node)
  if (known !== undefined) return known
  const n =
    node.type === 'file'
      ? node.name.endsWith('.md') ? 1 : 0
      : (node.children ?? []).reduce((sum, c) => sum + (isHiddenFolder(c) ? 0 : countNotesCached(c)), 0)
  countCache.set(node, n)
  return n
}

/**
 * The rows the tree shows: the root's folders, each open folder's sub-folders and then its notes
 * (A to Z, up to `limit` until it is in `showAll`). Folders that start with an underscore are left out
 * unless `showHidden` is set; then they are listed and marked `system`.
 */
export function treeRows(
  root: NoteTreeNode | null | undefined,
  open: ReadonlySet<string>,
  showAll: ReadonlySet<string> = new Set(),
  limit: number = FOLDER_NOTE_LIMIT,
  showHidden = false,
): TreeRow[] {
  const rows: TreeRow[] = []
  const walk = (parent: NoteTreeNode, depth: number) => {
    const kids = parent.children ?? []
    const folders = kids.filter((c) => c.type === 'dir' && (showHidden || !isHiddenFolder(c))).sort(byName)
    for (const f of folders) {
      const isOpen = open.has(f.path)
      rows.push({ kind: 'folder', path: f.path, name: f.name, depth, open: isOpen, count: countNotesCached(f), node: f, system: isHiddenFolder(f) })
      if (isOpen) walk(f, depth + 1)
    }
    if (depth === 0) return // the root's own notes are not part of the tree
    const notes = kids.filter((c) => c.type === 'file').sort(byName)
    const shown = showAll.has(parent.path) ? notes : notes.slice(0, limit)
    for (const n of shown) rows.push({ kind: 'note', path: n.path, name: noteName(n.path), depth })
    if (shown.length < notes.length) {
      rows.push({ kind: 'more', path: `more:${parent.path}`, folder: parent.path, name: parent.name, depth, hiddenCount: notes.length - shown.length, total: notes.length })
    }
  }
  if (root) walk(root, 0)
  return rows
}

export type TreeKey = 'ArrowDown' | 'ArrowUp' | 'ArrowRight' | 'ArrowLeft' | 'Home' | 'End'
export type TreeMove = { focus: number } | { toggle: string; open: boolean } | null

/**
 * What a key does in the tree, by the standard tree pattern: arrows move between visible rows,
 * Right opens a closed folder or steps into an open one, Left closes an open folder or steps out
 * to its parent, Home and End go to the first and last row.
 */
export function treeMove(rows: readonly TreeRow[], index: number, key: TreeKey): TreeMove {
  const row = rows[index]
  if (!row || rows.length === 0) return null
  switch (key) {
    case 'ArrowDown':
      return index + 1 < rows.length ? { focus: index + 1 } : null
    case 'ArrowUp':
      return index > 0 ? { focus: index - 1 } : null
    case 'Home':
      return { focus: 0 }
    case 'End':
      return { focus: rows.length - 1 }
    case 'ArrowRight':
      if (row.kind !== 'folder') return null
      if (!row.open) return { toggle: row.path, open: true }
      return index + 1 < rows.length && rows[index + 1].depth > row.depth ? { focus: index + 1 } : null
    case 'ArrowLeft': {
      if (row.kind === 'folder' && row.open) return { toggle: row.path, open: false }
      for (let i = index - 1; i >= 0; i--) {
        if (rows[i].depth < row.depth) return { focus: i }
      }
      return null
    }
  }
}

/** The node at a path in the tree, or null. */
export function findNode(root: NoteTreeNode | null | undefined, path: string): NoteTreeNode | null {
  if (!root) return null
  if (root.path === path) return root
  for (const child of root.children ?? []) {
    if (path === child.path || path.startsWith(`${child.path}/`)) {
      const hit = findNode(child, path)
      if (hit) return hit
    }
  }
  return null
}

export interface VaultLocation {
  /** The note or folder path under Notes, empty on Vault home and on the pages that are not a path. */
  path: string
  /** The Trash or a note opened by id: a page of the Vault that is not a place in the tree. */
  special: 'trash' | 'id' | null
}

/** Reads the Vault path out of the address (`/notes/Notebooks/Projects/Shop`). */
export function vaultLocation(pathname: string): VaultLocation {
  const rest = pathname.replace(/^\/notes\/?/, '')
  if (rest === 'trash' || rest.startsWith('trash/')) return { path: '', special: 'trash' }
  if (rest.startsWith('n/')) return { path: '', special: 'id' }
  const decoded = rest
    .split('/')
    .filter(Boolean)
    .map((part) => {
      try {
        return decodeURIComponent(part)
      } catch {
        return part
      }
    })
    .join('/')
  return { path: decoded, special: null }
}

/** The notes just before and after `current` among a folder's notes, A to Z (digits in order, case ignored). */
export function siblingsAround<T extends { name: string; path: string; isDir: boolean }>(entries: readonly T[], current: string): { prev: T | null; next: T | null } {
  const notes = entries.filter((e) => !e.isDir && e.name.endsWith('.md')).sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }))
  const i = notes.findIndex((e) => e.path === current)
  if (i < 0) return { prev: null, next: null }
  return { prev: notes[i - 1] ?? null, next: notes[i + 1] ?? null }
}

export interface SiblingItem {
  name: string
  path: string
  isDir: boolean
  /** Notes inside, for a folder. */
  count: number
}

/**
 * The folders beside a folder (or the notes beside a note), for the menu on a crumb: the other
 * children of its parent, of the same kind, A to Z. Underscore folders are left out.
 */
export function siblingsOf(root: NoteTreeNode | null | undefined, path: string): { parent: string; items: SiblingItem[] } {
  const here = findNode(root, path)
  const parent = parentPath(path)
  const parentNode = parent === null ? root : findNode(root, parent)
  if (!here || !parentNode) return { parent: parent ?? '', items: [] }
  const items = (parentNode.children ?? [])
    .filter((c) => c.type === here.type && !isHiddenFolder(c))
    .sort(byName)
    .map((c) => ({ name: c.type === 'file' ? noteName(c.path) : c.name, path: c.path, isDir: c.type === 'dir', count: c.type === 'dir' ? countNotesCached(c) : 0 }))
  return { parent: parent ?? '', items }
}

/**
 * The folder to offer when a path is gone: the deepest folder above it that still exists, or Vault
 * home (an empty path) when none does.
 */
export function nearestFolder(root: NoteTreeNode | null | undefined, path: string): string {
  const above = ancestorPaths(path).filter((p) => p !== NOTEBOOKS_ROOT)
  for (let i = above.length - 1; i >= 0; i--) {
    if (findNode(root, above[i])?.type === 'dir') return above[i]
  }
  return ''
}

/** Whether a tree has a folder or note at this path. */
export const existsInTree = (root: NoteTreeNode | null | undefined, path: string): boolean => findNode(root, path) !== null

export type HistoryAction = 'PUSH' | 'REPLACE' | 'BACK' | 'FORWARD' | 'GO'

/**
 * Tracks how far forward the person's history reaches. The browser knows how to go back, but not
 * whether Forward would do anything, so the furthest index seen is kept: a new page (PUSH) cuts off
 * what lay ahead, and moving back or forward keeps it.
 */
export function furthestIndex(furthest: number, action: HistoryAction, index: number): number {
  return action === 'PUSH' ? index : Math.max(furthest, index)
}
