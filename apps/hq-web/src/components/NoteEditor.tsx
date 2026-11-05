import { useState, useEffect, useCallback } from 'react'
import { MarkdownViewer } from './MarkdownViewer'
import { t } from '~/strings'
import { createNote, getNote, updateNote, type NoteConflict } from '~/lib/vaultApi'
import { keepBothTitle } from '~/lib/noteConflict'
import { keepUnsaved, onBeforeReload, takeUnsaved } from '~/lib/version'

interface NoteEditorProps {
    content: string
    filePath: string
    base?: string
    onSaved?: () => void
}

export function NoteEditor({ content, filePath, base, onSaved }: NoteEditorProps) {
    const [mode, setMode] = useState<'edit' | 'preview'>('edit')
    const [saving, setSaving] = useState(false)
    const [dirty, setDirty] = useState(false)
    const [conflict, setConflict] = useState<NoteConflict | null>(null)
    const [failed, setFailed] = useState(false)
    const [base0, setBase] = useState(base)

    useEffect(() => setBase(base), [base])

    // Strip frontmatter for editing — server handles preservation on save
    const bodyOnly = content.replace(/^---[\s\S]*?---\n?/, '')
    const [editContent, setEditContent] = useState(bodyOnly)

    useEffect(() => {
        const newBody = content.replace(/^---[\s\S]*?---\n?/, '')
        const kept = takeUnsaved(`note:${filePath}`)
        setEditContent(kept ?? newBody)
        setDirty(kept !== null && kept !== newBody)
    }, [content, filePath])

    // A reload for a newer HQ must not lose what has not been saved.
    useEffect(() => {
        if (!dirty) return
        return onBeforeReload(() => keepUnsaved(`note:${filePath}`, editContent))
    }, [dirty, filePath, editContent])

    const handleChange = (value: string) => {
        setEditContent(value)
        setDirty(true)
    }

    const handleSave = useCallback(async () => {
        if (!dirty || saving) return
        setSaving(true)
        setFailed(false)
        try {
            const result = await updateNote(filePath, editContent, base0)
            if (result.saved) {
                setDirty(false)
                if (result.hash) setBase(result.hash)
                onSaved?.()
            } else {
                setConflict(result)
            }
        } catch {
            setFailed(true)
        } finally {
            setSaving(false)
        }
    }, [dirty, saving, filePath, editContent, base0, onSaved])

    // Their version replaces the text here; Keep both saves this text as a second note next to it.
    const seeTheirs = async () => {
        const theirs = await getNote(filePath)
        setEditContent(theirs.content.replace(/^---[\s\S]*?---\n?/, ''))
        setBase(theirs.hash)
        setDirty(false)
        setConflict(null)
        onSaved?.()
    }
    const keepBoth = async () => {
        const folder = filePath.includes('/') ? filePath.split('/').slice(0, -1).join('/') : undefined
        const title = keepBothTitle(filePath, t('note_conflict_your_version'))
        try {
            await createNote(title, editContent, folder, crypto.randomUUID())
            await seeTheirs()
        } catch {
            setFailed(true)
        }
    }

    // Cmd+S / Ctrl+S to save
    useEffect(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault()
                handleSave()
            }
        }
        window.addEventListener('keydown', onKeyDown)
        return () => window.removeEventListener('keydown', onKeyDown)
    }, [handleSave])

    // Warn before navigating away with unsaved changes
    useEffect(() => {
        const handler = (e: BeforeUnloadEvent) => {
            if (dirty) { e.preventDefault() }
        }
        window.addEventListener('beforeunload', handler)
        return () => window.removeEventListener('beforeunload', handler)
    }, [dirty])

    return (
        <div className="flex flex-col h-full min-h-0">
            {/* Toolbar */}
            <div
                className="flex items-center justify-between px-1 py-2 flex-shrink-0 gap-2"
            >
                {/* Mode toggle — glass pills */}
                <div
                    className="flex rounded-xl p-0.5"
                    style={{ background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.04)' }}
                >
                    <button
                        onClick={() => setMode('edit')}
                        className="px-3 py-1.5 text-[10px] tracking-wider uppercase font-mono font-bold transition-all rounded-lg"
                        style={{
                            color: mode === 'edit' ? 'var(--accent-green)' : 'var(--text-dim)',
                            background: mode === 'edit' ? 'rgba(0,255,163,0.08)' : 'transparent',
                        }}
                    >
                        Edit
                    </button>
                    <button
                        onClick={() => setMode('preview')}
                        className="px-3 py-1.5 text-[10px] tracking-wider uppercase font-mono font-bold transition-all rounded-lg"
                        style={{
                            color: mode === 'preview' ? 'var(--accent-green)' : 'var(--text-dim)',
                            background: mode === 'preview' ? 'rgba(0,255,163,0.08)' : 'transparent',
                        }}
                    >
                        Preview
                    </button>
                </div>

                {/* Save area */}
                <div className="flex items-center gap-2">
                    <span className="text-[9px] font-mono hidden sm:inline" style={{ color: 'var(--text-dim)', opacity: 0.5 }}>
                        {editContent.split(/\s+/).filter(Boolean).length} words / {editContent.split('\n').length} lines
                    </span>
                    {dirty && (
                        <span className="text-[10px] font-mono flex items-center gap-1.5" style={{ color: 'var(--accent-amber)' }}>
                            <span className="w-1.5 h-1.5 rounded-full inline-block" style={{ background: 'var(--accent-amber)' }} />
                            unsaved
                        </span>
                    )}
                    <button
                        onClick={handleSave}
                        disabled={!dirty || saving}
                        className="px-3 py-1.5 rounded-xl text-xs font-mono font-bold transition-all disabled:opacity-30"
                        style={{
                            background: dirty ? 'rgba(0,255,163,0.12)' : 'rgba(255,255,255,0.04)',
                            color: dirty ? 'var(--accent-green)' : 'var(--text-dim)',
                            border: dirty ? '1px solid rgba(0,255,163,0.2)' : '1px solid rgba(255,255,255,0.06)',
                        }}
                    >
                        {saving ? 'Saving...' : 'Save'}
                    </button>
                </div>
            </div>

            {conflict && (
                <div role="alert" className="glass-card rounded-xl px-4 py-3 mb-2 flex flex-col gap-2 text-xs" style={{ color: 'var(--text-primary)' }}>
                    <p className="font-bold">{t('note_conflict_title')}</p>
                    <p style={{ color: 'var(--text-dim)' }}>{conflict.by ? t('note_conflict_by', { name: conflict.by }) : t('note_conflict_line')}</p>
                    <div className="flex gap-2">
                        <button onClick={() => void seeTheirs()} className="px-3 py-1.5 rounded-lg glass-card">{t('note_conflict_see_theirs')}</button>
                        <button onClick={() => void keepBoth()} className="px-3 py-1.5 rounded-lg glass-card">{t('note_conflict_keep_both')}</button>
                    </div>
                </div>
            )}
            {failed && <p role="alert" className="text-xs px-1 pb-2" style={{ color: 'var(--accent-amber)' }}>{t('fail_save_note_no_reply')}</p>}

            {/* Content area */}
            <div className="flex-1 min-h-0 overflow-y-auto">
                {mode === 'edit' ? (
                    <textarea
                        value={editContent}
                        onChange={(e) => handleChange(e.target.value)}
                        className="w-full h-full min-h-[400px] px-4 py-3 text-sm outline-none resize-none rounded-xl"
                        style={{
                            background: 'rgba(255,255,255,0.02)',
                            border: '1px solid rgba(255,255,255,0.06)',
                            color: 'var(--text-primary)',
                            fontFamily: 'var(--font-mono)',
                            fontSize: '13px',
                            lineHeight: '1.7',
                            caretColor: 'var(--accent-green)',
                        }}
                        spellCheck={false}
                    />
                ) : (
                    <MarkdownViewer content={editContent} activePath={filePath} />
                )}
            </div>

            {/* Mobile save bar */}
            <div
                className="md:hidden flex-shrink-0 flex items-center justify-between px-4 py-3"
                style={{ borderTop: '1px solid rgba(255,255,255,0.05)' }}
            >
                <div className="flex items-center gap-3">
                    <span className="text-[10px] font-mono" style={{ color: 'var(--text-dim)' }}>
                        {dirty ? 'Unsaved changes' : 'All changes saved'}
                    </span>
                    <span className="text-[9px] font-mono" style={{ color: 'var(--text-dim)', opacity: 0.6 }}>
                        {editContent.split(/\s+/).filter(Boolean).length}w / {editContent.split('\n').length}L
                    </span>
                </div>
                <button
                    onClick={handleSave}
                    disabled={!dirty || saving}
                    className="px-4 py-2 rounded-xl text-xs font-mono font-bold transition-all disabled:opacity-30"
                    style={{
                        background: dirty ? 'rgba(0,255,163,0.15)' : 'rgba(255,255,255,0.04)',
                        color: dirty ? 'var(--accent-green)' : 'var(--text-dim)',
                    }}
                >
                    {saving ? 'Saving...' : 'Save'}
                </button>
            </div>
        </div>
    )
}
