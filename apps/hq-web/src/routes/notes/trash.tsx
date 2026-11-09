import { createFileRoute } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { readTrashed, restoreNote, type TrashedNote } from '~/lib/vaultApi'
import { trashQuery, vaultKeys } from '~/lib/queries'
import { toastAlert, toastNotice } from '~/lib/toast'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { BackLink } from '~/components/vault/NoteViews'
import { folderTrail, noteName } from '~/components/vault/noteRows'
import { t, tn } from '~/strings'
import '~/components/vault/notes.css'

const NOTEBOOKS = 'Notebooks'

export const Route = createFileRoute('/notes/trash')({
    component: TrashPage,
    ssr: false,
})

/** A deleted note: where it was, how long it has left, and a way to read it or bring it back. Its text is shown as plain text. */
export function TrashRow({ note, onRestored }: { note: TrashedNote; onRestored: () => void }) {
    const [text, setText] = useState<string | null>(null)
    const [restoring, setRestoring] = useState(false)
    const place = folderTrail(note.original_path) || t('notes_your_notes')

    const toggle = async () => {
        if (text !== null) {
            setText(null)
            return
        }
        try {
            setText((await readTrashed(note.trash_path)).content)
        } catch {
            toastAlert(t('trash_read_failed'))
        }
    }

    const restore = async () => {
        setRestoring(true)
        try {
            const { path } = await restoreNote(note.trash_path)
            toastNotice(t('trash_restored_to', { place: folderTrail(path) || t('notes_your_notes') }))
            onRestored()
        } catch {
            toastAlert(t('trash_restore_failed'))
            setRestoring(false)
        }
    }

    const left = note.days_left > 0 ? tn('trash_days_left', note.days_left) : t('trash_last_day')
    return (
        <div className="an-trash-item">
            <div className="a-li">
                <div className="a-li-main">
                    <b>{noteName(note.original_path)}</b>
                    <span className="a-body-sm a-muted">
                        {t('trash_line', { place, date: note.trashed_on })} · {left}
                    </span>
                </div>
                <button type="button" className="a-btn compact quiet" aria-expanded={text !== null} onClick={() => void toggle()}>
                    {text !== null ? t('trash_hide') : t('trash_view')}
                </button>
                <button type="button" className="a-btn compact secondary" disabled={restoring} onClick={() => void restore()}>
                    {t('trash_restore')}
                </button>
            </div>
            {text !== null && <pre className="an-trash-text">{text}</pre>}
        </div>
    )
}

function TrashPage() {
    const queryClient = useQueryClient()
    const trash = useQuery(trashQuery)
    const items = trash.data?.items ?? []
    return (
        <Panes
            detailLabel={t('notes_trash')}
            detail={
                <>
                    <PaneHead title={t('notes_trash')} back={<BackLink path={NOTEBOOKS} />} />
                    <div className="an-home">
                        {trash.data && <p className="a-body-sm a-muted">{t('trash_intro', { days: trash.data.retention_days })}</p>}
                        {trash.isError && <p role="alert" className="a-body-sm a-muted">{t('trash_load_failed')}</p>}
                        {trash.data && items.length === 0 && <p className="a-body-sm a-muted">{t('trash_empty')}</p>}
                        {items.length > 0 && (
                            <div className="a-set" style={{ margin: 0 }}>
                                {items.map((note) => (
                                    <TrashRow
                                        key={note.trash_path}
                                        note={note}
                                        onRestored={() => void queryClient.invalidateQueries({ queryKey: vaultKeys.all })}
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                </>
            }
        />
    )
}
