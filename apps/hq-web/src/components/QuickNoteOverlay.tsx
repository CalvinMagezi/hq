import { useState, useEffect } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { X, Save, FileText } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { vaultKeys } from '~/lib/queries'
import { createNote, getFolderList } from '~/lib/vaultApi'

export function QuickNoteOverlay() {
    const [open, setOpen] = useState(false)
    const [title, setTitle] = useState('')
    const [content, setContent] = useState('')
    const [folder, setFolder] = useState('Notebooks/Inbox')
    const [folders, setFolders] = useState<string[]>(['Notebooks/Inbox'])
    const [saving, setSaving] = useState(false)
    const queryClient = useQueryClient()

    useEffect(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            // Cmd+N or Ctrl+N to open
            if ((e.metaKey || e.ctrlKey) && e.key === 'n') {
                e.preventDefault()
                setOpen(true)
            }
            if (e.key === 'Escape') setOpen(false)
        }
        const onOpen = () => setOpen(true)
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
        try {
            await createNote(title, content, folder)
            setOpen(false)
            setTitle('')
            setContent('')
            setFolder('Notebooks/Inbox')
            queryClient.invalidateQueries({ queryKey: vaultKeys.all })
        } catch (err) {
            console.error('Failed to create note', err)
        } finally {
            setSaving(false)
        }
    }

    return (
        <AnimatePresence>
            {open && (
                <>
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed inset-0 bg-black/50 backdrop-blur-sm z-50"
                        onClick={() => setOpen(false)}
                    />
                    {/* Centered by this wrapper because framer-motion owns the panel's transform. */}
                    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 pointer-events-none">
                        <motion.div
                            initial={{ opacity: 0, scale: 0.95, y: 20 }}
                            animate={{ opacity: 1, scale: 1, y: 0 }}
                            exit={{ opacity: 0, scale: 0.95, y: 20 }}
                            className="pointer-events-auto w-full max-w-xl rounded-xl shadow-2xl overflow-hidden flex flex-col"
                            style={{
                                maxHeight: '80vh',
                                background: 'var(--bg-solid-surface)',
                                border: '1px solid var(--border)',
                            }}
                        >
                            <div className="flex items-center justify-between px-4 py-3" style={{ borderBottom: '1px solid var(--border)' }}>
                                <div className="flex items-center gap-2">
                                    <FileText className="w-4 h-4" style={{ color: 'var(--accent-green)' }} />
                                    <span className="text-sm font-mono font-bold tracking-wide" style={{ color: 'var(--text-primary)' }}>Quick Note</span>
                                </div>
                                <button onClick={() => setOpen(false)} className="transition-colors" style={{ color: 'var(--text-dim)' }}>
                                    <X className="w-5 h-5" />
                                </button>
                            </div>

                            <div className="p-4 flex flex-col gap-4 flex-1 overflow-y-auto">
                                <div>
                                    <input
                                        autoFocus
                                        type="text"
                                        placeholder="Note Title"
                                        value={title}
                                        onChange={e => setTitle(e.target.value)}
                                        className="w-full rounded-lg px-4 py-3 outline-none transition-colors font-mono font-medium"
                                        style={{
                                            background: 'rgba(255,255,255,0.03)',
                                            border: '1px solid var(--border)',
                                            color: 'var(--text-primary)',
                                            fontSize: '16px',
                                            caretColor: 'var(--accent-green)',
                                        }}
                                    />
                                </div>

                                {/* Folder picker */}
                                <div>
                                    <select
                                        value={folder}
                                        onChange={e => setFolder(e.target.value)}
                                        className="w-full rounded-lg px-4 py-2.5 text-xs outline-none transition-colors font-mono"
                                        style={{
                                            background: 'rgba(255,255,255,0.03)',
                                            border: '1px solid var(--border)',
                                            color: 'var(--text-dim)',
                                        }}
                                    >
                                        {folders.map(f => (
                                            <option key={f} value={f}>{f}</option>
                                        ))}
                                    </select>
                                </div>

                                <div className="flex-1 min-h-[200px]">
                                    <textarea
                                        placeholder="Start typing your note..."
                                        value={content}
                                        onChange={e => setContent(e.target.value)}
                                        className="w-full h-full min-h-[200px] rounded-lg px-4 py-3 text-sm outline-none transition-colors resize-none font-mono"
                                        style={{
                                            background: 'rgba(255,255,255,0.03)',
                                            border: '1px solid var(--border)',
                                            color: 'var(--text-primary)',
                                            fontSize: '16px',
                                            lineHeight: '1.6',
                                            caretColor: 'var(--accent-green)',
                                        }}
                                        onKeyDown={e => {
                                            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                                                e.preventDefault()
                                                handleSave()
                                            }
                                        }}
                                    />
                                </div>
                            </div>

                            <div className="flex items-center justify-between px-4 py-3" style={{ borderTop: '1px solid var(--border)' }}>
                                <div className="text-xs font-mono flex items-center gap-2" style={{ color: 'var(--text-dim)' }}>
                                    Save with <kbd className="px-1.5 rounded" style={{ background: 'var(--bg-elevated)', border: '1px solid var(--border)' }}>Cmd+Enter</kbd>
                                </div>
                                <button
                                    onClick={handleSave}
                                    disabled={saving || (!title && !content)}
                                    className="flex items-center gap-2 disabled:opacity-30 disabled:cursor-not-allowed px-4 py-2 rounded-xl text-sm font-mono font-bold transition-all"
                                    style={{
                                        background: 'rgba(0,255,163,0.15)',
                                        color: 'var(--accent-green)',
                                        border: '1px solid rgba(0,255,163,0.25)',
                                    }}
                                >
                                    {saving ? 'Saving...' : (
                                        <>
                                            <Save className="w-4 h-4" /> Save
                                        </>
                                    )}
                                </button>
                            </div>
                        </motion.div>
                    </div>
                </>
            )}
        </AnimatePresence>
    )
}
