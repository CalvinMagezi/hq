// The comments and blocks of a note as last read from HQ, kept on this device so the Comments sheet still lists
// them (and can pick a block to comment on) with no connection. One copy per note and per HQ, never on a
// shared computer, and nothing over the size cap.
import { idb } from './offlineCache'
import { currentHqId, keyFor, PLACE_CAP_BYTES, type KeyValue } from './savedCopy'
import { isSharedComputer } from './sharedComputer'
import type { SuiteBlock, SuiteComment } from './suiteApi'

export interface CommentCopy {
  comments?: SuiteComment[]
  blocks?: SuiteBlock[]
  savedAt: number
}

const NAME = 'hq-suite-comments'
const keyOf = (path: string, hqId: string) => keyFor(`${NAME}:${path}`, hqId)

/** Keeps what was just read. `part` replaces only the half it names. */
export async function saveCommentCopy(path: string, part: Partial<Pick<CommentCopy, 'comments' | 'blocks'>>, kv: KeyValue = idb, now: () => number = Date.now): Promise<void> {
  const id = currentHqId()
  if (!id || isSharedComputer()) return
  try {
    const before = await readCommentCopy(path, kv)
    const next: CommentCopy = { ...before, ...part, savedAt: now() }
    const raw = JSON.stringify(next)
    if (raw.length > PLACE_CAP_BYTES) return
    await kv.set(keyOf(path, id), raw)
  } catch {
    // A browser that refuses storage keeps nothing; the sheet then reads from HQ only.
  }
}

export async function readCommentCopy(path: string, kv: KeyValue = idb): Promise<CommentCopy | null> {
  const id = currentHqId()
  if (!id || isSharedComputer()) return null
  try {
    const raw = await kv.get(keyOf(path, id))
    return raw ? (JSON.parse(raw) as CommentCopy) : null
  } catch {
    return null
  }
}
