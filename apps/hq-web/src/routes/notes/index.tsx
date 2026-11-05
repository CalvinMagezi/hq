import { createFileRoute } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { togglePinNote, type VaultSignals } from '~/lib/vaultApi'
import { pinnedNotesQuery, vaultSignalsQuery } from '~/lib/queries'
import { HomeSections } from '~/components/vault/HomeSections'
import { GetStartedCard } from '~/components/vault/GetStartedCard'
import { BriefingCard } from '~/components/tasks/verification/BriefingCard'
import { fetchTasksClient } from '~/lib/tasksApi'
import { inProgressTasks, sectionState } from '~/lib/homeSections'
import { useWS } from '~/context/WebSocketContext'
import { SignalCard } from '~/components/SignalCard'
import { t } from '~/strings'
import { Plus } from 'lucide-react'

export const Route = createFileRoute('/notes/')({
    component: VaultHome,
    // Data comes from the saved copy first, then the Rust API, so render client-side.
    ssr: false,
})

const EMPTY_SIGNALS: VaultSignals = { recent: [], work: [], review: [], activity: [] }

function InlineSearch() {
    return (
        <button
            type="button"
            className="a-input w-full text-left"
            onClick={() => window.dispatchEvent(new CustomEvent('hq:open-search', { detail: { scope: 'notes' } }))}
        >
            <span className="a-muted">{t('notes_search_your_notes')}</span>
        </button>
    )
}

function VaultHome() {
    const signalsQuery = useQuery(vaultSignalsQuery)
    const signals = signalsQuery.data ?? EMPTY_SIGNALS
    const pinnedQuery = useQuery({ ...pinnedNotesQuery, select: (d) => d.notes })
    const tasksQuery = useQuery({
        queryKey: ['tasks', 'in_progress'],
        queryFn: () => fetchTasksClient({ status: 'in_progress' }),
        select: (d) => inProgressTasks(d.tasks),
    })
    const queryClient = useQueryClient()
    const { subscribe } = useWS()

    useEffect(() => subscribe((msg) => {
        if (msg.type === 'task:sync') queryClient.invalidateQueries({ queryKey: ['tasks', 'in_progress'] })
    }), [subscribe, queryClient])

    return (
        <div className="h-full overflow-y-auto overflow-x-hidden">
            <div className="max-w-3xl mx-auto px-4 sm:px-6 py-8 sm:py-12 w-full">
                <div className="mb-10 flex items-stretch gap-2">
                    <InlineSearch />
                    <button
                        type="button"
                        onClick={() => window.dispatchEvent(new Event('hq:open-quick-note'))}
                        className="flex items-center gap-2 px-4 rounded-2xl glass-card text-xs flex-shrink-0"
                        style={{ color: 'var(--text-dim)' }}
                        title="New note"
                        aria-label="New note"
                    >
                        <Plus className="w-4 h-4" />
                        <span className="hidden sm:inline">New note</span>
                    </button>
                </div>

                <GetStartedCard />
                <BriefingCard variant="home" />

                <HomeSections
                    pinned={sectionState(pinnedQuery)}
                    recent={sectionState({ ...signalsQuery, data: signalsQuery.data?.recent })}
                    tasks={sectionState(tasksQuery)}
                    onKeepPin={(path) => void togglePinNote(path, true).then(() => pinnedQuery.refetch())}
                    onNotNow={(path) => void togglePinNote(path, false).then(() => pinnedQuery.refetch())}
                    onRetryPinned={() => pinnedQuery.refetch()}
                    onRetryRecent={() => signalsQuery.refetch()}
                    onRetryTasks={() => tasksQuery.refetch()}
                />

                <details className="mb-10 glass-card rounded-xl overflow-hidden">
                    <summary className="cursor-pointer px-4 py-3 text-[11px] tracking-widest uppercase font-bold" style={{ color: 'var(--text-dim)' }}>
                        System Activity
                    </summary>
                    <div className="px-4 pb-4 grid grid-cols-1 gap-3">
                        {signals.activity.length > 0 ? signals.activity.map((note) => (
                            <SignalCard key={note.path} note={note} lane="activity" />
                        )) : (
                            <div className="text-[11px] " style={{ color: 'var(--text-dim)' }}>
                                No system activity found.
                            </div>
                        )}
                    </div>
                </details>

                <div className="text-center pt-4 pb-8">
                    <p className="text-[11px] " style={{ color: 'var(--text-dim)', opacity: 0.4 }}>
                        Press <kbd className="px-1.5 py-0.5 rounded-md mx-0.5" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)' }}>⌘/</kbd> for global search anywhere
                    </p>
                </div>
            </div>
        </div>
    )
}
