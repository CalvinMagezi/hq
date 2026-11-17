import { useEffect, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useQueryClient } from '@tanstack/react-query'
import { t, tn } from '~/strings'
import { Sheet } from '~/components/ui/Sheet'
import { moveFolder, moveNote } from '~/lib/vaultApi'
import { vaultKeys } from '~/lib/queries'
import { normalizeTarget, placeOf, type MoveKind } from '~/lib/moveNote'
import { toastUndo } from '~/lib/toast'

const PREVIEW_DELAY_MS = 400

/**
 * Moves or renames a note. Typing a new place asks HQ who links to it (a dry run), so the person sees what
 * follows before choosing; Move does it, and Undo on the toast moves it back, links and all.
 */
export function MoveNote({ path, kind = 'note', onClose }: { path: string; kind?: MoveKind; onClose: () => void }) {
  const [text, setText] = useState(path)
  const [linked, setLinked] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const navigate = useNavigate()
  const client = useQueryClient()
  const target = normalizeTarget(text, path, kind)
  const run = (from: string, to: string, dryRun: boolean) => (kind === 'folder' ? moveFolder(from, to, dryRun).then((r) => r.notes.length) : moveNote(from, to, dryRun).then((r) => r.referrers.length))

  // Who links here, asked once the typing pauses.
  useEffect(() => {
    setLinked(null)
    if (!target.path) return
    const to = target.path
    let live = true
    const timer = window.setTimeout(() => {
      run(path, to, true)
        .then((n) => live && setLinked(n))
        .catch(() => undefined)
    }, PREVIEW_DELAY_MS)
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [target.path, path, kind])

  const go = async () => {
    if (!target.path || busy) return
    const to = target.path
    setBusy(true)
    setError(null)
    try {
      await run(path, to, false)
      void client.invalidateQueries({ queryKey: vaultKeys.all })
      toastUndo(
        t('move_done', { place: placeOf(to) }),
        async () => {
          await run(to, path, false)
          void client.invalidateQueries({ queryKey: vaultKeys.all })
          void navigate({ to: '/notes/$', params: { _splat: path } })
        },
        t('move_undone'),
      )
      onClose()
      void navigate({ to: '/notes/$', params: { _splat: to } })
    } catch (e) {
      setError(t('move_failed', { why: e instanceof Error ? e.message : '' }))
      setBusy(false)
    }
  }

  return (
    <Sheet
      title={t(kind === 'folder' ? 'move_folder_title' : 'move_title')}
      placement="center"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="a-btn secondary" disabled={busy} onClick={onClose}>
            {t('suite_edit_cancel')}
          </button>
          <button type="button" className="a-btn" disabled={busy || !target.path} onClick={() => void go()}>
            {busy ? t('move_busy') : t('move_action')}
          </button>
        </>
      }
    >
      <div className="an-move">
        <label className="a-label" htmlFor="an-move-path">
          {t('move_label')}
        </label>
        <input
          id="an-move-path"
          className="an-move-input"
          autoFocus
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              void go()
            }
          }}
        />
        <p className="a-body-sm a-muted">{t('move_line')}</p>
        {target.problem && <p role="status" className="a-body-sm a-warn-t">{t(target.problem)}</p>}
        {!target.problem && linked !== null && (
          <p role="status" className="a-body-sm">
            {kind === 'folder' ? (linked === 0 ? t('move_folder_empty') : tn('move_folder_notes', linked)) : linked === 0 ? t('move_no_links') : tn('move_links', linked)}
          </p>
        )}
        {error && <p role="alert" className="a-body-sm a-warn-t">{error}</p>}
      </div>
    </Sheet>
  )
}
