// Links to a note by its id (web wave 4 W22, resolutions C-10): `/notes/n/{id}` keeps working when the
// note is renamed or moved, unlike its folder path.
import { HqHttpError, hqFetch, hqJson } from './hqAuth'

export interface NoteLink {
  id: string
  path: string
  /** The version on disk once the id is in the note: the editor's next save starts from it. */
  hash: string
}

export const noteLinkUrl = (origin: string, id: string) => `${origin.replace(/\/$/, '')}/notes/n/${encodeURIComponent(id)}`

/** The note's id, which HQ adds to the note the first time a link is copied. */
export function mintNoteLink(path: string): Promise<NoteLink> {
  return hqJson('/api/note/link', 'POST', { path })
}

/** Where the note with this id lives now, or null when no note has it. */
export async function notePathById(id: string): Promise<string | null> {
  const res = await hqFetch(`/api/note/by-id/${encodeURIComponent(id)}`)
  if (res.status === 404) return null
  if (!res.ok) throw new HqHttpError(`${res.status} ${res.statusText}`, res.status)
  const body = await res.json()
  return typeof body.path === 'string' ? body.path : null
}

/** Copy link: puts the id link on the clipboard and reports the note's new version. */
export async function copyNoteLink(
  path: string,
  deps: { mint: (path: string) => Promise<NoteLink>; copy: (text: string) => Promise<boolean>; origin: string },
): Promise<{ copied: boolean; link: NoteLink }> {
  const link = await deps.mint(path)
  return { copied: await deps.copy(noteLinkUrl(deps.origin, link.id)), link }
}
