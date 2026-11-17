import { useCallback, useEffect, useRef, useState } from 'react'
import { t } from '~/strings'
import { updateNote } from '~/lib/vaultApi'
import { toastAlert } from '~/lib/toast'
import { editable } from '~/lib/hqmdEdit'

export type SaveState = 'idle' | 'saving' | 'saved'

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
 */
export function useSuiteEdit({ path, content, hash, onSaved }: Options) {
  const [text, setText] = useState(content)
  const [state, setState] = useState<SaveState>('idle')
  const latest = useRef(content)
  const base = useRef(hash)
  const queue = useRef<Promise<void>>(Promise.resolve())

  // A newer copy from HQ replaces what is shown, unless a save is on its way.
  useEffect(() => {
    latest.current = content
    base.current = hash
    setText(content)
  }, [content, hash])

  const canEdit = !!path && editable(text)

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
        try {
          const saved = await updateNote(path, next, base.current)
          if (saved.saved) {
            base.current = saved.hash ?? base.current
            setState('saved')
            onSaved?.()
          } else {
            latest.current = before
            setText(before)
            setState('idle')
            toastAlert(t('suite_edit_conflict'))
            onSaved?.()
          }
        } catch {
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

  return { text, canEdit, apply, state }
}
