import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Plus, Search, Trash2 } from 'lucide-react'
import { togglePinNote, type NoteTreeNode, type PinnedNote, type RecentFile, type VaultSignals } from '~/lib/vaultApi'
import { noteTreeQuery, pinnedNotesQuery, vaultSignalsQuery } from '~/lib/queries'
import { sectionState, type SectionState } from '~/lib/homeSections'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { HomeSections } from '~/components/vault/HomeSections'
import { BriefingCard } from '~/components/tasks/verification/BriefingCard'
import { topFolders } from '~/components/vault/noteRows'
import { t } from '~/strings'
import '~/components/vault/notes.css'

export const Route = createFileRoute('/notes/')({
    component: VaultHome,
    // Data comes from the saved copy first, then the Rust API, so render client-side.
    ssr: false,
})

const NOTEBOOKS = 'Notebooks'

const openSearch = () => window.dispatchEvent(new CustomEvent('hq:open-search', { detail: { scope: 'notes' } }))
const openNewNote = () => window.dispatchEvent(new Event('hq:open-quick-note'))

interface NotesHomeViewProps {
    pinned: SectionState<PinnedNote>
    recent: SectionState<RecentFile>
    folders: NoteTreeNode[]
    onKeepPin?: (path: string) => void
    onNotNow?: (path: string) => void
    onRetryPinned: () => void
    onRetryRecent: () => void
    /** Extra cards above the sections (the first-run checklist, the briefing). */
    intro?: React.ReactNode
    /** The way to the Trash, passed in because it needs the router. */
    trashLink?: React.ReactNode
}

/** Notes home: find, the folders to open, what is pinned, what changed lately. */
export function NotesHomeView({ pinned, recent, folders, onKeepPin, onNotNow, onRetryPinned, onRetryRecent, intro, trashLink }: NotesHomeViewProps) {
    return (
        <Panes
            detailLabel={t('notes_your_notes')}
            detail={
                <>
                    <PaneHead title={t('notes_your_notes')}>
                        {trashLink}
                        <button type="button" className="a-btn compact" onClick={openNewNote} title={t('notes_new_note_key')}>
                            <Plus className="a-ic" aria-hidden="true" />
                            {t('notes_new_note')}
                            <kbd className="a-kbd an-kbd">{t('notes_new_note_key')}</kbd>
                        </button>
                    </PaneHead>
                    <div className="an-home">
                        <button type="button" className="a-input an-findpill" onClick={openSearch}>
                            <Search className="a-ic" aria-hidden="true" />
                            <span>{t('notes_search_your_notes')}</span>
                            <kbd className="a-kbd">{t('search_kbd')}</kbd>
                        </button>
                        {folders.length > 0 && (
                            <div className="a-chips" role="group" aria-label={t('notes_folders')}>
                                {folders.map((f) => (
                                    <Link key={f.path} to="/notes/$" params={{ _splat: f.path }} className="a-chip">
                                        {f.name}
                                    </Link>
                                ))}
                            </div>
                        )}
                        {intro}
                        <HomeSections
                            pinned={pinned}
                            recent={recent}
                            onKeepPin={onKeepPin}
                            onNotNow={onNotNow}
                            onRetryPinned={onRetryPinned}
                            onRetryRecent={onRetryRecent}
                        />
                    </div>
                </>
            }
        />
    )
}

const EMPTY_SIGNALS: VaultSignals = { recent: [], work: [], review: [], activity: [] }

function VaultHome() {
    const signalsQuery = useQuery(vaultSignalsQuery)
    const pinnedQuery = useQuery({ ...pinnedNotesQuery, select: (d) => d.notes })
    const tree = useQuery(noteTreeQuery(NOTEBOOKS))
    const signals = signalsQuery.data ?? EMPTY_SIGNALS

    return (
        <NotesHomeView
            pinned={sectionState(pinnedQuery)}
            recent={sectionState({ ...signalsQuery, data: signals.recent })}
            folders={topFolders(tree.data?.tree)}
            onKeepPin={(path) => void togglePinNote(path, true).then(() => pinnedQuery.refetch())}
            onNotNow={(path) => void togglePinNote(path, false).then(() => pinnedQuery.refetch())}
            onRetryPinned={() => pinnedQuery.refetch()}
            onRetryRecent={() => signalsQuery.refetch()}
            trashLink={
                <Link to="/notes/trash" className="a-btn compact secondary">
                    <Trash2 className="a-ic" aria-hidden="true" />
                    {t('notes_trash')}
                </Link>
            }
            intro={
                <>
                    <BriefingCard variant="home" />
                </>
            }
        />
    )
}
