import { useState, useEffect, useCallback, useRef, type MutableRefObject } from 'react'
import { Bold, Heading2, Link2, List, ListChecks } from 'lucide-react'
import { Link, useBlocker } from '@tanstack/react-router'
import { MarkdownViewer } from './MarkdownViewer'
import { Dialog, workingLabel } from '~/components/ui/Dialog'
import { t } from '~/strings'
import { displayZone } from '~/lib/time'
import { conflictWhere, stripFrontmatter } from '~/lib/noteConflict'
import { keepBoth as keepBothNote, liveNoteApi, saveNote, seeTheirVersion, type NoteApi, type SaveStep, type TheirVersion } from '~/lib/noteSave'
import type { NoteConflict } from '~/lib/vaultApi'
import '~/components/vault/notes.css'
import { keepUnsaved, onBeforeReload, takeUnsaved } from '~/lib/version'

interface NoteEditorProps {
    content: string
    filePath: string
    base?: string
    onSaved?: () => void
    /** The page asks here before it leaves the editor: unsaved text gets the Save your changes? question. */
    leaveRef?: MutableRefObject<((go: () => void) => void) | null>
    api?: NoteApi
}

interface Asking {
    go: () => void
    stay: () => void
}

function clock(at?: string): string {
    const ms = at ? Date.parse(at) : Date.now()
    return new Intl.DateTimeFormat(undefined, { timeZone: displayZone(), hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(Number.isNaN(ms) ? Date.now() : ms)
}

function whereWords(by: string | undefined, kind: 'cap' | 'low'): string | null {
    const where = conflictWhere(by)
    if (where === 'unknown') return null
    if (where === 'helper') return t(kind === 'cap' ? 'note_where_helper' : 'note_from_helper')
    return t(kind === 'cap' ? 'note_where_hq' : 'note_from_hq')
}

/** What a formatting button does to the text: wrap the selection, or start the line with a marker. */
export type Format = 'heading' | 'bold' | 'italic' | 'bullets' | 'checklist' | 'link'

/** The new text and selection after one formatting button, so the textarea stays plain markdown. */
export function applyFormat(text: string, start: number, end: number, format: Format): { text: string; start: number; end: number } {
    const wrap = (open: string, close: string) => {
        const next = text.slice(0, start) + open + text.slice(start, end) + close + text.slice(end)
        return { text: next, start: start + open.length, end: end + open.length }
    }
    const lead = (marker: string) => {
        const at = text.lastIndexOf('\n', start - 1) + 1
        return { text: text.slice(0, at) + marker + text.slice(at), start: start + marker.length, end: end + marker.length }
    }
    if (format === 'bold') return wrap('**', '**')
    if (format === 'italic') return wrap('*', '*')
    if (format === 'link') return wrap('[[', ']]')
    if (format === 'heading') return lead('## ')
    if (format === 'bullets') return lead('- ')
    return lead('- [ ] ')
}

export function NoteEditor({ content, filePath, base, onSaved, leaveRef, api = liveNoteApi }: NoteEditorProps) {
    const [mode, setMode] = useState<'edit' | 'preview'>('edit')
    const [saving, setSaving] = useState(false)
    const [dirty, setDirty] = useState(false)
    const [conflict, setConflict] = useState<NoteConflict | null>(null)
    const [theirs, setTheirs] = useState<TheirVersion | null>(null)
    const [kept, setKept] = useState<{ title: string; path: string } | null>(null)
    const [failed, setFailed] = useState(false)
    const [base0, setBase] = useState(base)
    const [asking, setAsking] = useState<Asking | null>(null)
    const [busy, setBusy] = useState(false)

    useEffect(() => setBase(base), [base])

    // Strip frontmatter for editing; the server handles preservation on save
    const bodyOnly = stripFrontmatter(content)
    const [editContent, setEditContent] = useState(bodyOnly)

    // Keyed on the body, so a frontmatter-only change (Copy link adding the note's id, a pin) keeps
    // what is being typed; the new base still arrives through `base`.
    useEffect(() => {
        const keptText = takeUnsaved(`note:${filePath}`)
        setEditContent(keptText ?? bodyOnly)
        setDirty(keptText !== null && keptText !== bodyOnly)
    }, [bodyOnly, filePath])

    // A reload for a newer HQ must not lose what has not been saved.
    useEffect(() => {
        if (!dirty) return
        return onBeforeReload(() => keepUnsaved(`note:${filePath}`, editContent))
    }, [dirty, filePath, editContent])

    const areaRef = useRef<HTMLTextAreaElement | null>(null)
    const format = (kind: Format) => {
        const area = areaRef.current
        if (!area) return
        const next = applyFormat(editContent, area.selectionStart, area.selectionEnd, kind)
        setEditContent(next.text)
        setDirty(true)
        requestAnimationFrame(() => {
            area.focus()
            area.setSelectionRange(next.start, next.end)
        })
    }

    const handleChange = (value: string) => {
        setEditContent(value)
        setDirty(true)
    }

    const handleSave = useCallback(async (): Promise<SaveStep | null> => {
        if (!dirty || saving || conflict) return null
        setSaving(true)
        setFailed(false)
        const step = await saveNote(api, filePath, editContent, base0)
        if (step.kind === 'saved') {
            setDirty(false)
            if (step.hash) setBase(step.hash)
            onSaved?.()
        } else if (step.kind === 'conflict') {
            setConflict(step.conflict)
        } else {
            setFailed(true)
        }
        setSaving(false)
        return step
    }, [api, dirty, saving, conflict, filePath, editContent, base0, onSaved])

    const seeTheirs = async () => {
        const seen = await seeTheirVersion(api, filePath, editContent)
        if (seen) setTheirs(seen)
        else setFailed(true)
    }

    // Your text becomes a new note beside theirs; theirs then stays on screen as the note.
    const keepBoth = async () => {
        setFailed(false)
        const done = await keepBothNote(api, filePath, editContent, t('note_conflict_your_version'), () => crypto.randomUUID())
        if (!done.ok) {
            setFailed(true)
            return
        }
        setEditContent(done.theirs.text)
        setBase(done.theirs.hash)
        setDirty(false)
        setConflict(null)
        setTheirs(null)
        setKept({ title: done.title, path: done.path })
        onSaved?.()
    }

    useEffect(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault()
                void handleSave()
            } else if ((e.metaKey || e.ctrlKey) && (e.key === 'b' || e.key === 'i') && document.activeElement === areaRef.current) {
                e.preventDefault()
                format(e.key === 'b' ? 'bold' : 'italic')
            }
        }
        window.addEventListener('keydown', onKeyDown)
        return () => window.removeEventListener('keydown', onKeyDown)
    }, [handleSave, format])

    // Closing the tab asks the browser's own question; its words cannot be set.
    useEffect(() => {
        const handler = (e: BeforeUnloadEvent) => {
            if (dirty) { e.preventDefault() }
        }
        window.addEventListener('beforeunload', handler)
        return () => window.removeEventListener('beforeunload', handler)
    }, [dirty])

    const dirtyRef = useRef(dirty)
    dirtyRef.current = dirty
    const blocker = useBlocker({ shouldBlockFn: () => dirtyRef.current, withResolver: true, enableBeforeUnload: false })
    useEffect(() => {
        if (blocker.status === 'blocked') setAsking({ go: blocker.proceed, stay: blocker.reset })
    }, [blocker.status, blocker.proceed, blocker.reset])

    useEffect(() => {
        if (!leaveRef) return
        leaveRef.current = (go) => {
            if (dirtyRef.current) setAsking({ go, stay: () => undefined })
            else go()
        }
        return () => { leaveRef.current = null }
    }, [leaveRef])

    const discard = () => {
        takeUnsaved(`note:${filePath}`)
        setDirty(false)
        const go = asking?.go
        setAsking(null)
        // The blocker reads the ref, so let the clean state land before it lets the navigation through.
        dirtyRef.current = false
        go?.()
    }
    const saveAndLeave = async () => {
        if (!asking) return
        setBusy(true)
        const step = await handleSave()
        setBusy(false)
        const next = asking
        setAsking(null)
        if (step?.kind === 'saved') {
            dirtyRef.current = false
            next.go()
        } else {
            next.stay()
        }
    }
    const keepEditing = () => {
        asking?.stay()
        setAsking(null)
    }

    const status = conflict ? t('note_conflict_not_saved') : saving ? t('note_save_saving') : failed ? t('note_save_failed') : dirty ? t('note_save_unsaved') : t('note_save_saved')
    const words = editContent.split(/\s+/).filter(Boolean).length
    const lines = editContent.split('\n').length
    const time = clock(conflict?.at)
    const from = whereWords(conflict?.by, 'cap')
    const fromLow = whereWords(conflict?.by, 'low')

    return (
        <div className="flex flex-col h-full min-h-0">
            <div className="flex items-center justify-between px-1 py-2 flex-shrink-0 gap-2">
                <div className="a-chips" style={{ display: 'flex', gap: 8 }}>
                    <button type="button" className={`a-chip${mode === 'edit' ? ' is-on' : ''}`} onClick={() => setMode('edit')}>
                        {t('note_editor_edit')}
                    </button>
                    <button type="button" className={`a-chip${mode === 'preview' ? ' is-on' : ''}`} onClick={() => setMode('preview')}>
                        {t('note_editor_preview')}
                    </button>
                </div>

                <div className="flex items-center gap-2">
                    <span className="a-muted a-body-sm hidden sm:inline">{t('note_editor_counts', { words, lines })}</span>
                    <span className="an-save" role="status" style={{ color: dirty || failed || conflict ? 'var(--warn)' : undefined }}>
                        {status}
                    </span>
                    <button type="button" className="a-btn" onClick={() => void handleSave()} disabled={!dirty || saving || !!conflict}>
                        {t('quick_note_save')}
                    </button>
                </div>
            </div>

            {conflict && !theirs && (
                <div role="alert" className="an-conf">
                    <h2 className="a-h3">{t('note_conflict_title')}</h2>
                    <p className="a-body-sm a-muted">
                        {from ? t('note_conflict_line', { where: from, time }) : t('note_conflict_line_unknown', { time })}
                    </p>
                    <div className="a-card-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                        <button type="button" className="a-btn secondary" onClick={() => void seeTheirs()}>{t('note_conflict_see_theirs')}</button>
                        <button type="button" className="a-btn secondary" onClick={() => void keepBoth()}>{t('note_conflict_keep_both')}</button>
                    </div>
                </div>
            )}
            {conflict && theirs && (
                <section aria-label={t('note_conflict_theirs_head')} className="an-conf">
                    <h2 className="a-h3">{t('note_conflict_theirs_head')}</h2>
                    <p className="a-body-sm a-muted">
                        {fromLow ? t('note_conflict_theirs_from', { where: fromLow, time }) : t('note_conflict_theirs_from_unknown', { time })}
                    </p>
                    <p className="a-body-sm a-muted">{t('note_conflict_differ', { n: theirs.differ })}</p>
                    <textarea readOnly aria-readonly="true" className="a-input" rows={8} value={theirs.text} style={{ fontFamily: 'var(--font-mono)' }} />
                    <div className="a-card-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                        <button type="button" className="a-btn secondary" onClick={() => setTheirs(null)}>{t('note_conflict_back_to_mine')}</button>
                        <button type="button" className="a-btn secondary" onClick={() => void keepBoth()}>{t('note_conflict_keep_both')}</button>
                    </div>
                </section>
            )}
            {kept && (
                <div role="status" className="a-banner">
                    <span>{t('note_conflict_both_kept', { title: kept.title })}</span>
                    <Link to="/notes/$" params={{ _splat: kept.path }} className="a-link">{t('note_conflict_open')}</Link>
                </div>
            )}
            {failed && (
                <div role="alert" className="a-banner">
                    <span style={{ color: 'var(--warn)' }}>{t('fail_save_changes_no_reply')}</span>
                    <button type="button" className="a-link" onClick={() => void handleSave()}>{t('note_editor_retry')}</button>
                </div>
            )}

            {mode === 'edit' && (
                <div className="an-fmt" role="toolbar" aria-label={t('notes_fmt_label')}>
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_heading')} onClick={() => format('heading')}><Heading2 className="a-ic" aria-hidden="true" /></button>
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_bold')} onClick={() => format('bold')}><Bold className="a-ic" aria-hidden="true" /></button>
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_italic')} onClick={() => format('italic')}><i aria-hidden="true">I</i></button>
                    <span className="an-fmt-sep" aria-hidden="true" />
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_bullets')} onClick={() => format('bullets')}><List className="a-ic" aria-hidden="true" /></button>
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_checklist')} onClick={() => format('checklist')}><ListChecks className="a-ic" aria-hidden="true" /></button>
                    <button type="button" className="a-ib" aria-label={t('notes_fmt_link')} onClick={() => format('link')}><Link2 className="a-ic" aria-hidden="true" /></button>
                </div>
            )}

            <div className="flex-1 min-h-0 overflow-y-auto">
                {mode === 'edit' ? (
                    <textarea
                        ref={areaRef}
                        aria-label={t('notes_note_text')}
                        value={editContent}
                        onChange={(e) => handleChange(e.target.value)}
                        className="an-ed"
                        spellCheck={false}
                    />
                ) : (
                    <MarkdownViewer content={editContent} activePath={filePath} />
                )}
            </div>

            <p className="a-muted a-body-sm hidden md:block" style={{ padding: '4px 4px' }}>{t('note_editor_shortcut')}</p>

            {asking && (
                <Dialog
                    kind="red"
                    title={t('note_leave_title')}
                    body={t('note_leave_body', { note: (filePath.split('/').pop() ?? '').replace(/\.md$/, '') })}
                    keepLabel={t('note_leave_keep')}
                    actLabel={t('note_leave_discard')}
                    busy={busy}
                    busyLabel={workingLabel()}
                    third={{ label: t('leave_note_save'), onPick: () => void saveAndLeave() }}
                    onKeep={keepEditing}
                    onAct={discard}
                />
            )}
        </div>
    )
}
