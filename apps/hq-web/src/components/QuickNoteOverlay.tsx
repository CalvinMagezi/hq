import { useState, useEffect } from 'react'
import { Check, ChevronRight, Folder } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { vaultKeys } from '~/lib/queries'
import { createNote, getFolderList } from '~/lib/vaultApi'
import { t } from '~/strings'
import { Sheet } from '~/components/ui/Sheet'
import { folderChoices } from '~/components/vault/noteRows'
import '~/components/vault/notes.css'
import { opensQuickNote, savesQuickNote } from '~/lib/quickNote'

export function QuickNoteOverlay() {
    const [open, setOpen] = useState(false)
    const [title, setTitle] = useState('')
    const [content, setContent] = useState('')
    const [folder, setFolder] = useState('Notebooks/Inbox')
    const [folders, setFolders] = useState<string[]>(['Notebooks/Inbox'])
    const [saving, setSaving] = useState(false)
    const [failed, setFailed] = useState(false)
    const [picking, setPicking] = useState(false)
    const [filter, setFilter] = useState('')
    const [clientId, setClientId] = useState(() => crypto.randomUUID())
    const queryClient = useQueryClient()

    useEffect(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            if (opensQuickNote(e)) {
                e.preventDefault()
                setOpen(true)
            }
        }
        const onOpen = (e: Event) => {
            const where = (e as CustomEvent<{ folder?: string } | undefined>).detail?.folder
            if (where) setFolder(where)
            setOpen(true)
        }
        window.addEventListener('keydown', onKeyDown)
        window.addEventListener('hq:open-quick-note', onOpen)
        return () => {
            window.removeEventListener('keydown', onKeyDown)
            window.removeEventListener('hq:open-quick-note', onOpen)
        }
    }, [])

    // Load folder list when overlay opens
    useEffect(() => {
        if (open) {
            getFolderList().then((res) => setFolders(res.folders))
        }
    }, [open])

    const handleSave = async () => {
        if (!title && !content) return
        setSaving(true)
        setFailed(false)
        try {
            await createNote(title, content, folder, clientId)
            setClientId(crypto.randomUUID())
            setOpen(false)
            setTitle('')
            setContent('')
            setFolder('Notebooks/Inbox')
            setPicking(false)
            queryClient.invalidateQueries({ queryKey: vaultKeys.all })
        } catch (err) {
            console.error('Failed to create note', err)
            setFailed(true)
        } finally {
            setSaving(false)
        }
    }

    if (!open) return null

    const close = () => {
        setOpen(false)
        setPicking(false)
    }
    const choices = folderChoices(folders).filter((f) => f.path.toLowerCase().includes(filter.trim().toLowerCase()))

    if (picking) {
        return (
            <Sheet
                title={t('notes_folder_where_new')}
                onClose={() => setPicking(false)}
                footer={<button type="button" className="a-btn secondary" onClick={() => setPicking(false)}>{t('notes_cancel')}</button>}
            >
                <div className="a-field">
                    <input
                        type="search"
                        className="a-input"
                        aria-label={t('notes_folder_where_new')}
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                    />
                </div>
                <div role="menu" aria-label={t('notes_folder_where_new')}>
                    {choices.map((f) => (
                        <button
                            key={f.path}
                            type="button"
                            role="menuitemradio"
                            aria-checked={f.path === folder}
                            className="an-folder"
                            style={{ paddingLeft: 16 + f.depth * 20 }}
                            onClick={() => {
                                setFolder(f.path)
                                setPicking(false)
                            }}
                        >
                            <Folder className="a-ic" aria-hidden="true" />
                            <span>{f.name}</span>
                            {f.path === folder && <Check className="a-ic" aria-hidden="true" />}
                        </button>
                    ))}
                </div>
            </Sheet>
        )
    }

    return (
        <Sheet
            title={t('quick_note_title')}
            onClose={close}
            busy={saving}
            footer={
                <>
                    <span className="a-body-sm a-muted" style={{ marginRight: 'auto' }}>
                        {failed ? <span role="alert">{t('quick_note_fail')}</span> : t('quick_note_hint')}
                    </span>
                    <button type="button" className="a-btn secondary" onClick={close} disabled={saving}>{t('notes_cancel')}</button>
                    <button type="button" className="a-btn" onClick={handleSave} disabled={saving || (!title && !content)}>
                        {saving ? t('quick_note_saving') : t('quick_note_save')}
                    </button>
                </>
            }
        >
            <div className="a-field">
                <input
                    autoFocus
                    type="text"
                    className="a-input"
                    aria-label={t('notes_title')}
                    placeholder={t('notes_title')}
                    value={title}
                    onChange={(e) => setTitle(e.target.value)}
                />
            </div>
            <button type="button" className="an-folder" aria-haspopup="menu" onClick={() => setPicking(true)}>
                <Folder className="a-ic" aria-hidden="true" />
                <span>{t('notes_saving_to', { folder: folder.replace(/^Notebooks\//, '') })}</span>
                <ChevronRight className="a-ic" aria-hidden="true" />
            </button>
            <div className="a-field">
                <textarea
                    className="a-input"
                    rows={8}
                    aria-label={t('notes_note_text')}
                    placeholder={t('notes_start_writing')}
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    onKeyDown={(e) => {
                        if (savesQuickNote(e)) {
                            e.preventDefault()
                            void handleSave()
                        }
                    }}
                />
            </div>
        </Sheet>
    )
}
