// Suite edits made while HQ cannot be reached wait in the outbox and are sent when it is back, against the
// hash the page had when the first of them was made. If the note moved on in between, nothing is written:
// the person's version is kept as a conflict they can see and copy out, never silently lost or overwritten.
import { t } from '~/strings'
import { newClientId, type Outbox, type OutboxEntry } from './outbox'
import type { SaveResult } from './vaultApi'

/** What waits for one note: the whole note text after the edits, and the hash they were made against. */
export interface PendingNote {
  path: string
  base?: string
  text: string
}

const isPending = (e: OutboxEntry) => e.kind === 'note' && !e.waited && (e.payload as PendingNote | undefined)?.path !== undefined
/** A kept conflict is an entry marked waited, so the drain never sends it. */
const isConflict = (e: OutboxEntry) => e.kind === 'note' && e.waited === true && e.last_reason === 'conflict'

/**
 * Keeps an edit for later. Later edits to the same note replace the text but keep the first hash, so the
 * whole run is checked once against what the page first saw.
 */
export async function queueSuiteEdit(outbox: Outbox, hqId: string, edit: PendingNote, now: () => number = Date.now): Promise<{ ok: true } | { ok: false; message: string }> {
  const earlier = (await outbox.list(hqId)).find((e) => isPending(e) && (e.payload as PendingNote).path === edit.path)
  const base = earlier ? (earlier.payload as PendingNote).base : edit.base
  if (earlier) await outbox.remove(hqId, earlier.client_id)
  const added = await outbox.add({
    client_id: newClientId(),
    kind: 'note',
    hq_id: hqId,
    payload: { path: edit.path, base, text: edit.text } satisfies PendingNote,
    created_at: earlier?.created_at ?? now(),
    attempts: 0,
  })
  return added.ok ? { ok: true } : { ok: false, message: added.message }
}

export type Update = (path: string, text: string, base?: string) => Promise<SaveResult>

export interface Replay {
  sent: string[]
  conflicts: string[]
}

/** Sends what waits, oldest first. A refusal that is not a network failure ends the wait for that note. */
export async function replaySuiteEdits(outbox: Outbox, hqId: string, update: Update, isNetworkError: (e: unknown) => boolean): Promise<Replay> {
  const result: Replay = { sent: [], conflicts: [] }
  await outbox.drain(
    hqId,
    async (entry) => {
      if (!isPending(entry)) return 'gone'
      const p = entry.payload as PendingNote
      try {
        const res = await update(p.path, p.text, p.base)
        if (res.saved) {
          result.sent.push(p.path)
          return 'sent'
        }
      } catch (e) {
        return isNetworkError(e) ? 'retry' : 'gone'
      }
      // 409: the note changed. Keep the person's text beside it instead of dropping it.
      result.conflicts.push(p.path)
      await outbox.add({ ...entry, client_id: newClientId(), waited: true, last_reason: 'conflict' })
      return 'gone'
    },
    undefined,
    'note',
  )
  return result
}

/** The notes whose waiting edits did not go through because the note changed. */
export async function suiteConflicts(outbox: Outbox, hqId: string): Promise<{ clientId: string; path: string; text: string }[]> {
  return (await outbox.list(hqId)).filter(isConflict).map((e) => ({ clientId: e.client_id, path: (e.payload as PendingNote).path, text: (e.payload as PendingNote).text }))
}

export async function waitingCount(outbox: Outbox, hqId: string, path?: string): Promise<number> {
  return (await outbox.list(hqId)).filter((e) => isPending(e) && (path === undefined || (e.payload as PendingNote).path === path)).length
}

/** The sentence for a note whose offline edit was not applied. */
export const conflictLine = (path: string) => t('suite_offline_conflict', { name: path.split('/').pop()?.replace(/\.md$/, '') ?? path })

/** A save that reached HQ makes older waiting copies of the same note stale: forget them. */
export async function dropWaiting(outbox: Outbox, hqId: string, path: string): Promise<void> {
  for (const e of await outbox.list(hqId)) if (isPending(e) && (e.payload as PendingNote).path === path) await outbox.remove(hqId, e.client_id)
}

/** The person keeps or drops a conflict text. */
export const removeConflict = (outbox: Outbox, hqId: string, clientId: string) => outbox.remove(hqId, clientId)
