// Moving and renaming a note: what the person types becomes a path inside the notes, or a plain reason it cannot.
import type { StringKey } from '~/strings'

export type Target = { path: string; problem: null } | { path: null; problem: StringKey }

/** The vault's top folder, which every note sits under. */
export const NOTES_ROOT = 'Notebooks'

/** A typed path as a note path: slashes tidied, `.md` added, and a reason when it cannot be one. */
export function normalizeTarget(input: string, current: string): Target {
  const typed = input.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/').replace(/^\/+/, '')
  if (/[<>:"|?*\u0000-\u001f]/.test(typed) || typed.split('/').some((p) => p === '..' || p === '.')) return { path: null, problem: 'move_bad_chars' }
  if (!typed.startsWith(`${NOTES_ROOT}/`)) return { path: null, problem: 'move_bad_start' }
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
