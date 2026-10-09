import { createFileRoute } from '@tanstack/react-router'
import { useRef, useCallback, useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { VaultSignals } from '~/lib/vaultApi'
import { pinnedNotesQuery, vaultSignalsQuery } from '~/lib/queries'
import { HomeSections } from '~/components/vault/HomeSections'
import { fetchTasksClient } from '~/lib/tasksApi'
import { inProgressTasks, sectionState } from '~/lib/homeSections'
import { useWS } from '~/context/WebSocketContext'
import { SignalCard } from '~/components/SignalCard'
import { useVaultSearch } from '~/components/useVaultSearch'
import { Plus } from 'lucide-react'

export const Route = createFileRoute('/vault/')({
    component: VaultHome,
    // Data comes from the saved copy first, then the Rust API, so render client-side.
    ssr: false,
})

const EMPTY_SIGNALS: VaultSignals = { recent: [], work: [], review: [], activity: [] }

function InlineSearch() {
    const inputRef = useRef<HTMLInputElement>(null)
    const blur = useCallback(() => inputRef.current?.blur(), [])
    const { query, setQuery, results, isLoading, selectedIndex, openResult, onKeyDown } = useVaultSearch(blur)

    return (
        <div className="relative w-full">
            <div
                className="flex items-center gap-3 px-4 py-3.5 rounded-2xl transition-all glass-card"
                style={{
                    boxShadow: query
                        ? '0 0 0 1px rgba(0, 173, 238, 0.3), 0 8px 32px rgba(0,0,0,0.3), 0 0 60px rgba(0, 173, 238, 0.06)'
                        : '0 4px 24px rgba(0,0,0,0.3), inset 0 1px 0 var(--glass-shine)',
                    borderColor: query ? 'rgba(0, 173, 238, 0.35)' : undefined,
                }}
            >
                <span className="text-base flex-shrink-0" style={{ color: 'var(--text-dim)', opacity: 0.6 }}>⌕</span>
                <input
                    ref={inputRef}
                    type="text"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    onKeyDown={onKeyDown}
                    placeholder="Search vault..."
                    className="flex-1 bg-transparent outline-none text-base sm:text-sm "
                    style={{ color: 'var(--text-primary)' }}
                />
                {isLoading && (
                    <div className="w-4 h-4 rounded-full border-2 border-t-transparent animate-spin flex-shrink-0" style={{ borderColor: 'var(--accent-green)', borderTopColor: 'transparent' }} />
                )}
                <kbd className="hidden sm:inline text-[11px] px-1.5 py-0.5 rounded-md flex-shrink-0" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)', color: 'var(--text-dim)' }}>
                    ⌘/
                </kbd>
            </div>

            {/* Results dropdown — glass */}
            {query && results.length > 0 && (
                <div
                    className="absolute z-40 top-full left-0 right-0 mt-2 rounded-2xl overflow-hidden max-h-[50vh] overflow-y-auto glass-heavy"
                    style={{
                        boxShadow: '0 16px 48px rgba(0, 0, 0, 0.5)',
                    }}
                >
                    {results.map((hit, idx) => (
                        <button
                            key={hit.notePath}
                            onClick={() => openResult(hit)}
                            className="w-full text-left px-4 py-3 flex flex-col gap-1 transition-all"
                            style={{
                                borderBottom: '1px solid rgba(255,255,255,0.04)',
                                background: idx === selectedIndex ? 'rgba(0, 173, 238, 0.08)' : 'transparent',
                                borderLeft: idx === selectedIndex ? '2px solid var(--accent-blue)' : '2px solid transparent',
                            }}
                        >
                            <div className="flex items-center justify-between gap-3">
                                <span className="text-sm font-bold truncate" style={{ color: 'var(--text-primary)' }}>
                                    {hit.title}
                                </span>
                                <span className="text-[11px] flex-shrink-0" style={{ color: 'var(--text-dim)' }}>
                                    {hit.notebook}
                                </span>
                            </div>
                            {hit.snippet && (
                                <p className="text-xs line-clamp-2 leading-relaxed" style={{ color: 'var(--text-dim)' }}>
                                    {hit.snippet}
                                </p>
                            )}
                            {hit.tags.length > 0 && (
                                <div className="flex gap-1">
                                    {hit.tags.slice(0, 4).map((t) => (
                                        <span key={t} className="text-[9px] px-1.5 py-0.5 rounded-md glass-tag" style={{ color: 'var(--text-dim)' }}>
                                            #{t}
                                        </span>
                                    ))}
                                </div>
                            )}
                        </button>
                    ))}
                    <div className="px-4 py-2 flex items-center justify-between text-[11px] " style={{ color: 'var(--text-dim)', borderTop: '1px solid rgba(255,255,255,0.04)' }}>
                        <span>{results.length} results</span>
                        <div className="flex items-center gap-3">
                            <span><kbd className="px-1 rounded" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)' }}>↑↓</kbd> navigate</span>
                            <span><kbd className="px-1 rounded" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)' }}>↵</kbd> open</span>
                        </div>
                    </div>
                </div>
            )}

            {query && !isLoading && results.length === 0 && (
                <div
                    className="absolute z-40 top-full left-0 right-0 mt-2 rounded-2xl overflow-hidden glass-heavy"
                    style={{ boxShadow: '0 16px 48px rgba(0, 0, 0, 0.5)' }}
                >
                    <div className="px-6 py-8 text-center text-sm " style={{ color: 'var(--text-dim)' }}>
                        No results for "{query}"
                    </div>
                </div>
            )}
        </div>
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

                <HomeSections
                    pinned={sectionState(pinnedQuery)}
                    recent={sectionState({ ...signalsQuery, data: signalsQuery.data?.recent })}
                    tasks={sectionState(tasksQuery)}
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
