// Moving and renaming a note: what the person types becomes a path inside the notes, or a plain reason it cannot.
import type { StringKey } from '~/strings'

export type Target = { path: string; problem: null } | { path: null; problem: StringKey }

/** The vault's top folder, which every note sits under. */
export const NOTES_ROOT = 'Notebooks'

export type MoveKind = 'note' | 'folder'

/** A typed path as a note path: slashes tidied, `.md` added, and a reason when it cannot be one. For a folder there is no `.md`. */
export function normalizeTarget(input: string, current: string, kind: MoveKind = 'note'): Target {
  const typed = input.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/').replace(/^\/+/, '')
  if (/[<>:"|?*\u0000-\u001f]/.test(typed) || typed.split('/').some((p) => p === '..' || p === '.')) return { path: null, problem: 'move_bad_chars' }
  if (!typed.startsWith(`${NOTES_ROOT}/`)) return { path: null, problem: 'move_bad_start' }
  if (kind === 'folder') {
    const folder = typed.replace(/\/+$/, '')
    if ((folder.split('/').pop() ?? '').trim() === '' || folder === NOTES_ROOT) return { path: null, problem: 'move_bad_name' }
    if (folder === current) return { path: null, problem: 'move_same' }
    if (folder.startsWith(`${current}/`)) return { path: null, problem: 'move_into_itself' }
    return { path: folder, problem: null }
  }
  const withExt = /\.md$/i.test(typed) ? typed : `${typed}.md`
  const name = withExt.split('/').pop() ?? ''
  if (name.replace(/\.md$/i, '').trim() === '') return { path: null, problem: 'move_bad_name' }
  if (withExt === current) return { path: null, problem: 'move_same' }
  return { path: withExt, problem: null }
}

/** "Notebooks/Projects/Plan.md" as the place a toast names: the folder, then the name. */
export function placeOf(path: string): string {
  return path.replace(new RegExp(`^${NOTES_ROOT}/`), '').replace(/\.md$/i, '')
}

/** What a drag carries: the kind and the path of the note or folder being moved. */
export interface Dragged {
  kind: MoveKind
  path: string
}

const lastPart = (p: string) => p.split('/').filter(Boolean).pop() ?? ''
const parentOf = (p: string) => p.split('/').slice(0, -1).join('/')

/**
 * Where a drop would put the thing: inside the folder, keeping its name. Null when it would do nothing or harm:
 * onto its own folder, into itself or something inside it, or into a system folder.
 */
export function dropTarget(drag: Dragged, folder: string): { from: string; to: string; kind: MoveKind } | null {
  if (!folder.startsWith(NOTES_ROOT) || folder.split('/').some((p) => p.startsWith('_'))) return null
  if (drag.path === NOTES_ROOT || drag.path.split('/').some((p) => p.startsWith('_'))) return null
  if (drag.kind === 'folder' && (folder === drag.path || folder.startsWith(`${drag.path}/`))) return null
  if (parentOf(drag.path) === folder) return null
  return { from: drag.path, to: `${folder}/${lastPart(drag.path)}`, kind: drag.kind }
}
