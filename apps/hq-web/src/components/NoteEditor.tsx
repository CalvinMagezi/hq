import { useState, useEffect, useCallback, useRef, type MutableRefObject } from 'react'
import { Link, useBlocker } from '@tanstack/react-router'
import { MarkdownViewer } from './MarkdownViewer'
import { Dialog, workingLabel } from '~/components/ui/Dialog'
import { t } from '~/strings'
import { displayZone } from '~/lib/time'
import { conflictWhere, stripFrontmatter } from '~/lib/noteConflict'
import { keepBoth as keepBothNote, liveNoteApi, saveNote, seeTheirVersion, type NoteApi, type SaveStep, type TheirVersion } from '~/lib/noteSave'
import type { NoteConflict } from '~/lib/vaultApi'
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

    // Strip frontmatter for editing — server handles preservation on save
    const bodyOnly = stripFrontmatter(content)
    const [editContent, setEditContent] = useState(bodyOnly)

    useEffect(() => {
        const newBody = stripFrontmatter(content)
        const keptText = takeUnsaved(`note:${filePath}`)
        setEditContent(keptText ?? newBody)
        setDirty(keptText !== null && keptText !== newBody)
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
            }
        }
        window.addEventListener('keydown', onKeyDown)
        return () => window.removeEventListener('keydown', onKeyDown)
    }, [handleSave])

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
                    <span className="a-body-sm" role="status" style={{ color: dirty || failed || conflict ? 'var(--warn)' : 'var(--ink-muted)' }}>
                        {status}
                    </span>
                    <button type="button" className="a-btn" onClick={() => void handleSave()} disabled={!dirty || saving || !!conflict}>
                        {t('quick_note_save')}
                    </button>
                </div>
            </div>

            {conflict && !theirs && (
                <div role="alert" className="a-card" style={{ marginBottom: 8 }}>
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
                <section aria-label={t('note_conflict_theirs_head')} className="a-card" style={{ marginBottom: 8 }}>
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

            <div className="flex-1 min-h-0 overflow-y-auto">
                {mode === 'edit' ? (
                    <textarea
                        value={editContent}
                        onChange={(e) => handleChange(e.target.value)}
                        className="a-input w-full h-full min-h-[400px] resize-none"
                        style={{ fontFamily: 'var(--font-mono)', fontSize: 13, lineHeight: 1.7 }}
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
