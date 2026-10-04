import { useState, useEffect, useCallback } from 'react'
import { MarkdownViewer } from './MarkdownViewer'
import { updateNote } from '~/lib/vaultApi'

interface NoteEditorProps {
    content: string
    filePath: string
    onSaved?: () => void
}

export function NoteEditor({ content, filePath, onSaved }: NoteEditorProps) {
    const [mode, setMode] = useState<'edit' | 'preview'>('edit')
    const [saving, setSaving] = useState(false)
    const [dirty, setDirty] = useState(false)

    // Strip frontmatter for editing — server handles preservation on save
    const bodyOnly = content.replace(/^---[\s\S]*?---\n?/, '')
    const [editContent, setEditContent] = useState(bodyOnly)

    useEffect(() => {
        const newBody = content.replace(/^---[\s\S]*?---\n?/, '')
        setEditContent(newBody)
        setDirty(false)
    }, [content])

    const handleChange = (value: string) => {
        setEditContent(value)
        setDirty(true)
    }

    const handleSave = useCallback(async () => {
        if (!dirty || saving) return
        setSaving(true)
        try {
            const result = await updateNote(filePath, editContent)
            if (result.success) {
                setDirty(false)
                onSaved?.()
            }
        } finally {
            setSaving(false)
        }
    }, [dirty, saving, filePath, editContent, onSaved])

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
