import { useState, type MouseEvent, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft, ChevronRight, Download, FileText, Folder, ListChecks, MessageSquare, MoreHorizontal, PanelRight, Pencil, Pin, PinOff, Eye, Plus } from 'lucide-react'
import type { DirEntry } from '~/lib/vaultApi'
import { currentVersion, fetchNoteVersions, reviewTag, whoWrote } from '~/lib/noteReview'
import { relTime } from '~/lib/time'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { whoText } from '~/components/notes/NoteHistory'
import { ReviewTag } from '~/components/notes/ReviewTag'
import { t, tn } from '~/strings'
import { FOLDER_NOTE_LIMIT, crumbsOf, noteName, parentPath } from './noteRows'
import './notes.css'

const NOTES_HOME = '/notes'

/** The back control of a pane head: up one folder, or to Notes home from a top-level path. */
export function BackLink({ path }: { path: string }) {
    const parent = parentPath(path)
    const name = parent ? noteName(parent) : t('notes_your_notes')
    const label = t('notes_back_to', { name })
    return parent ? (
        <Link to="/notes/$" params={{ _splat: parent }} className="a-ib a-back" aria-label={label}>
            <ArrowLeft className="a-ic" aria-hidden="true" />
        </Link>
    ) : (
        <Link to={NOTES_HOME} className="a-ib a-back" aria-label={label}>
            <ArrowLeft className="a-ic" aria-hidden="true" />
        </Link>
    )
}

/** Each folder above a path as a link that opens it, with Notes itself first. */
export function Crumbs({ path }: { path: string }) {
    const crumbs = crumbsOf(path)
    return (
        <nav className="an-crumbs" aria-label={t('notes_your_notes')}>
            <Link to={NOTES_HOME}>{t('notes_your_notes')}</Link>
            {crumbs.map((c) => (
                <span key={c.path} style={{ display: 'contents' }}>
                    <ChevronRight className="a-ic xs" aria-hidden="true" />
                    <Link to="/notes/$" params={{ _splat: c.path }}>{c.name}</Link>
                </span>
            ))}
        </nav>
    )
}

interface FolderViewProps {
    dirPath: string
    entries: DirEntry[]
    /** Notes inside each sub-folder, by path, once known. */
    counts?: Record<string, number>
    onNewHere: () => void
}

/** A folder: its sub-folders, then its notes A to Z. A long list stops at 25 until "See all". */
export function FolderView({ dirPath, entries, counts, onNewHere }: FolderViewProps) {
    const [all, setAll] = useState(false)
    const folders = entries.filter((e) => e.isDir)
    const notes = entries.filter((e) => !e.isDir).sort((a, b) => a.name.localeCompare(b.name))
    const shown = all ? notes : notes.slice(0, FOLDER_NOTE_LIMIT)
    return (
        <Panes
            detailLabel={noteName(dirPath)}
            detail={
                <>
                    <PaneHead title={noteName(dirPath)} back={<BackLink path={dirPath} />}>
                        <button type="button" className="a-btn compact" onClick={onNewHere}>
                            <Plus className="a-ic" aria-hidden="true" />
                            {t('notes_new_note_here')}
                        </button>
                    </PaneHead>
                    <div className="an-home">
                        <Crumbs path={`${dirPath}/x`} />
                        <p className="a-body-sm a-muted">{tn('notes_in_folder_count', entries.length)}</p>
                        {folders.length > 0 && (
                            <>
                                <p className="a-grp">
                                    {t('notes_folders')}
                                    <em className="a-count">{folders.length}</em>
                                </p>
                                <div className="an-rows">
                                    {folders.map((f) => (
                                        <Link key={f.path} to="/notes/$" params={{ _splat: f.path }} className="a-row">
                                            <span className="a-av" aria-hidden="true"><Folder className="a-ic" /></span>
                                            <span className="a-row-main"><b>{f.name}</b></span>
                                            {counts?.[f.path] !== undefined && <span className="a-body-sm a-muted">{counts[f.path]}</span>}
                                        </Link>
                                    ))}
                                </div>
                            </>
                        )}
                        <p className="a-grp">
                            {t('notes_a_to_z')}
                            <em className="a-count">{notes.length}</em>
                        </p>
                        {notes.length === 0 && <p className="a-body-sm a-muted">{t('notes_no_notes_yet')}</p>}
                        <div className="an-rows">
                            {shown.map((n) => (
                                <Link key={n.path} to="/notes/$" params={{ _splat: n.path }} className="a-row">
                                    <span className="a-av" aria-hidden="true"><FileText className="a-ic" /></span>
                                    <span className="a-row-main"><b>{noteName(n.path)}</b></span>
                                </Link>
                            ))}
                        </div>
                        {!all && FOLDER_NOTE_LIMIT < notes.length && (
                            <button type="button" className="a-btn secondary" onClick={() => setAll(true)}>
                                {t('notes_see_all', { n: notes.length })}
                            </button>
                        )}
                    </div>
                </>
            }
        />
    )
}

/** Where a note lives, who last saved it, and whether anyone checked it. */
export function NoteMeta({ path }: { path: string }) {
    const versions = useQuery({ queryKey: ['note-versions', path], queryFn: () => fetchNoteVersions(path), retry: false })
    const now = versions.data ? currentVersion(versions.data) : null
    return (
        <div className="an-meta">
            <Crumbs path={path} />
            {now && <span>{t('notes_saved_by', { who: whoText(whoWrote(now)), when: relTime(now.written_at) })}</span>}
            {versions.data && <ReviewTag kind={reviewTag(now)} />}
        </div>
    )
}

interface NoteBarProps {
    editing: boolean
    pinned: boolean
    pinning: boolean
    exporting: boolean
    exportOpen: boolean
    onEdit: () => void
    onAsk: () => void
    onTask: () => void
    onPin: () => void
    onExport: (e: MouseEvent<HTMLButtonElement>) => void
}

/** The note's actions in one row of chips: Edit first, then Ask HQ, Make a task, Pin and Export. */
export function NoteBar({ editing, pinned, pinning, exporting, exportOpen, onEdit, onAsk, onTask, onPin, onExport }: NoteBarProps) {
    return (
        <div className="a-chips an-bar" role="group" aria-label={t('notes_actions')}>
            <button type="button" className="a-btn compact" onClick={onEdit}>
                {editing ? <Eye className="a-ic" aria-hidden="true" /> : <Pencil className="a-ic" aria-hidden="true" />}
                {editing ? t('note_editor_view') : t('note_editor_edit')}
            </button>
            <button type="button" className="a-chip" onClick={onAsk}>
                <MessageSquare className="a-ic xs" aria-hidden="true" />
                {t('notes_ask_hq')}
            </button>
            <button type="button" className="a-chip" onClick={onTask}>
                <ListChecks className="a-ic xs" aria-hidden="true" />
                {t('notes_make_task')}
            </button>
            <button type="button" className="a-chip" aria-pressed={pinned} disabled={pinning} onClick={onPin}>
                {pinned ? <PinOff className="a-ic xs" aria-hidden="true" /> : <Pin className="a-ic xs" aria-hidden="true" />}
                {pinned ? t('pin_unpin') : t('notes_pin')}
            </button>
            <button type="button" className="a-chip" aria-haspopup="menu" aria-expanded={exportOpen} disabled={exporting} onClick={onExport}>
                <Download className="a-ic xs" aria-hidden="true" />
                {t('note_action_export')}
            </button>
        </div>
    )
}

interface NoteLayoutProps {
    filePath: string
    /** The folder's notes beside the open one, when the note has a folder. */
    list?: ReactNode
    side?: ReactNode
    sideOpen: boolean
    onToggleSide: () => void
    onMore: (e: MouseEvent<HTMLButtonElement>) => void
    bar?: ReactNode
    children: ReactNode
}

/** An open note: its head with More, the bar, the text, and About this note docked beside it. */
export function NoteLayout({ filePath, list, side, sideOpen, onToggleSide, onMore, bar, children }: NoteLayoutProps) {
    return (
        <Panes
            list={list}
            showDetail
            side={side}
            sideOpen={sideOpen}
            detailLabel={noteName(filePath)}
            detail={
                <>
                    <PaneHead title={noteName(filePath)} back={<BackLink path={filePath} />}>
                        {side !== undefined && (
                            <button type="button" className="a-ib a-side-toggle" aria-label={t('note_about_title')} aria-expanded={sideOpen} onClick={onToggleSide}>
                                <PanelRight className="a-ic" aria-hidden="true" />
                            </button>
                        )}
                        <button type="button" className="a-ib" aria-label={t('notes_more_for_note')} aria-haspopup="menu" onClick={onMore}>
                            <MoreHorizontal className="a-ic" aria-hidden="true" />
                        </button>
                    </PaneHead>
                    {bar}
                    {children}
                </>
            }
        />
    )
}

/** The notes beside the open one: the folder it lives in, as the list pane. */
export function SiblingList({ parent, entries, current }: { parent: string; entries: DirEntry[]; current: string }) {
    const notes = entries.filter((e) => !e.isDir).sort((a, b) => a.name.localeCompare(b.name))
    return (
        <>
            <PaneHead title={noteName(parent)} />
            <div className="an-rows">
                {notes.map((n) => (
                    <Link key={n.path} to="/notes/$" params={{ _splat: n.path }} className="a-row" aria-current={n.path === current ? 'true' : undefined}>
                        <span className="a-av" aria-hidden="true"><FileText className="a-ic" /></span>
                        <span className="a-row-main"><b>{noteName(n.path)}</b></span>
                    </Link>
                ))}
            </div>
        </>
    )
}
