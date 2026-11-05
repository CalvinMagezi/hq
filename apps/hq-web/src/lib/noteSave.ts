// The save, See their version and Keep both steps of a note that changed somewhere else (spec 03 X1 to
// X4). The editor only draws what these return, so the 409 flow runs the same in a test as on screen.

import { createNote, getNote, updateNote, type NoteConflict } from './vaultApi'
import { differingLines, keepBothTitle, stripFrontmatter } from './noteConflict'

export interface NoteApi {
  update: typeof updateNote
  get: typeof getNote
  create: typeof createNote
}

export const liveNoteApi: NoteApi = { update: updateNote, get: getNote, create: createNote }

export type SaveStep =
  | { kind: 'saved'; hash?: string }
  | { kind: 'conflict'; conflict: NoteConflict }
  | { kind: 'failed' }

/** Saves against `base`; a 409 is a conflict (nothing written), any other failure keeps the text. */
export async function saveNote(api: NoteApi, path: string, text: string, base?: string): Promise<SaveStep> {
  try {
    const res = await api.update(path, text, base)
    if (res.saved) return { kind: 'saved', hash: res.hash }
    return { kind: 'conflict', conflict: res }
  } catch {
    return { kind: 'failed' }
  }
}

export interface TheirVersion {
  text: string
  hash?: string
  differ: number
}

/** Their text next to yours, read only: nothing is replaced and nothing is saved. */
export async function seeTheirVersion(api: NoteApi, path: string, mine: string): Promise<TheirVersion | null> {
  try {
    const theirs = await api.get(path)
    const text = stripFrontmatter(theirs.content)
    return { text, hash: theirs.hash, differ: differingLines(mine, text) }
  } catch {
    return null
  }
}

export type KeepBoth =
  | { ok: true; title: string; path: string; theirs: TheirVersion }
  | { ok: false }

/** Your text becomes a new note "{title}, your version" beside theirs, with a client id of its own. */
export async function keepBoth(api: NoteApi, path: string, mine: string, suffix: string, newId: () => string): Promise<KeepBoth> {
  const folder = path.includes('/') ? path.split('/').slice(0, -1).join('/') : undefined
  const title = keepBothTitle(path, suffix)
  try {
    const made = await api.create(title, mine, folder, newId())
    const theirs = await seeTheirVersion(api, path, mine)
    if (!theirs) return { ok: false }
    return { ok: true, title, path: made.path, theirs }
  } catch {
    return { ok: false }
  }
}
