import { useCallback, useEffect, useRef, useState } from 'react'
import { t } from '~/strings'
import { updateNote } from '~/lib/vaultApi'
import { HqHttpError } from '~/lib/hqAuth'
import { toastAlert } from '~/lib/toast'
import { editable } from '~/lib/hqmdEdit'
import { useSharedConnection } from '~/lib/connection'
import { replayLive, suiteOutbox as outbox } from '~/lib/commentOutbox'
import { currentHqId } from '~/lib/savedCopy'
import { isSharedComputer } from '~/lib/sharedComputer'
import { dropWaiting, queueSuiteEdit, removeConflict, replaySuiteEdits, suiteConflicts, waitingCount } from '~/lib/suiteOutbox'

export type SaveState = 'idle' | 'saving' | 'saved' | 'queued'

/** An offline edit that was not applied because the note changed in the meantime; the text is kept for the person. */
export interface KeptConflict {
  clientId: string
  path: string
  text: string
}

interface Options {
  /** Where the note lives; without it the viewer only reads. */
  path?: string
  content: string
  hash?: string
  /** The note was saved: the page can load it again. */
  onSaved?: () => void
}

/**
 * The note text a suite viewer draws, with edits saved one at a time against the hash this page last saw.
 * An edit shows at once; if HQ says the note changed elsewhere, the edit is dropped and the latest text comes back.
 * When HQ cannot be reached the edit waits in the outbox (never on a shared computer) and is sent, against the
 * same hash, when it is back; if the note moved on, the edit is kept as a conflict instead of being written.
 */
export function useSuiteEdit({ path, content, hash, onSaved }: Options) {
  const [text, setText] = useState(content)
  const [state, setState] = useState<SaveState>('idle')
  const [conflict, setConflict] = useState<KeptConflict | null>(null)
  const latest = useRef(content)
  const base = useRef(hash)
  const queue = useRef<Promise<void>>(Promise.resolve())
  const connection = useSharedConnection()
  const waiting = useRef(false)

  // A newer copy from HQ replaces what is shown, unless an edit is waiting to be sent.
  useEffect(() => {
    if (waiting.current) return
    latest.current = content
    base.current = hash
    setText(content)
  }, [content, hash])

  const canEdit = !!path && editable(text)

  const refreshConflict = useCallback(async () => {
    const id = currentHqId()
    if (!id || !path) return
    const mine = (await suiteConflicts(outbox, id)).find((c) => c.path === path)
    setConflict(mine ?? null)
  }, [path])

  // Back online: send what waited. A conflict is shown, not written.
  useEffect(() => {
    const id = currentHqId()
    if (!path || !id) return
    void waitingCount(outbox, id, path).then((n) => {
      waiting.current = n > 0
      if (n > 0) setState('queued')
    })
    void refreshConflict()
    if (connection !== 'connected') return
    void replaySuiteEdits(outbox, id, updateNote, (e) => !(e instanceof HqHttpError)).then(async (r) => {
      waiting.current = (await waitingCount(outbox, id, path)) > 0
      if (r.sent.includes(path)) {
        setState('saved')
        onSaved?.()
      }
      if (r.conflicts.includes(path)) {
        setState('idle')
        toastAlert(t('suite_edit_conflict'))
        onSaved?.()
      }
      await refreshConflict()
      // Comments wait behind note edits, so the blocks they name are the ones HQ now has.
      const c = await replayLive()
      if (c.refused > 0) toastAlert(t('suite_comment_refused_toast'))
      if (c.sent.includes(path)) onSaved?.()
    })
  }, [connection, path, onSaved, refreshConflict])

  /** Applies one edit to the note. `change` returns the new text, or null when the edit is not safe. Returns whether it was taken. */
  const apply = useCallback(
    (change: (current: string) => string | null): boolean => {
      if (!path) return false
      const next = change(latest.current)
      if (next === null) {
        toastAlert(t('suite_edit_stale'))
        return false
      }
      if (next === latest.current) return true
      const before = latest.current
      latest.current = next
      setText(next)
      setState('saving')
      queue.current = queue.current.then(async () => {
        const id = currentHqId()
        try {
          const saved = await updateNote(path, next, base.current)
          if (saved.saved) {
            base.current = saved.hash ?? base.current
            if (id) await dropWaiting(outbox, id, path)
            waiting.current = false
            setState('saved')
            onSaved?.()
          } else {
            latest.current = before
            setText(before)
            setState('idle')
            toastAlert(t('suite_edit_conflict'))
            onSaved?.()
          }
        } catch (e) {
          // A refusal from HQ is not an outage: the edit goes back. An outage keeps it on this device.
          if (!(e instanceof HqHttpError) && id && !isSharedComputer()) {
            const kept = await queueSuiteEdit(outbox, id, { path, base: base.current, text: next })
            if (kept.ok) {
              waiting.current = true
              setState('queued')
              return
            }
            toastAlert(kept.message)
          }
          latest.current = before
          setText(before)
          setState('idle')
          toastAlert(t('suite_edit_failed'))
        }
      })
      return true
    },
    [path, onSaved],
  )

  const dismissConflict = useCallback(async () => {
    const id = currentHqId()
    if (id && conflict) await removeConflict(outbox, id, conflict.clientId)
    setConflict(null)
  }, [conflict])

  const baseHash = useCallback(() => base.current, [])
  return { text, canEdit, apply, state, conflict, dismissConflict, baseHash }
}
