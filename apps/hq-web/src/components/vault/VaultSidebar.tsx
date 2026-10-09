import { Link } from '@tanstack/react-router'
import { Pin, Folder, ChevronDown, PanelLeftClose, X } from 'lucide-react'
import type { NoteTreeNode, PinnedNote } from '~/lib/vaultApi'
import { PinnedCard } from './PinnedCard'
import { SidebarTreeSkeleton, TreeNode, countFiles } from './VaultTree'

export type Section = 'Notebooks' | '_system' | '_logs'

interface Props {
    query: string
    setQuery: (q: string) => void
    onCollapse: () => void
    pinned: PinnedNote[]
    pinnedExpanded: boolean
    togglePinned: () => void
    activePath: string | null
    onNavigate: () => void
    onUnpin: (path: string) => void
    tree: NoteTreeNode | null
    treeLoading: boolean
    filteredTree: NoteTreeNode | null
    section: Section
    setSection: (s: Section) => void
    expandedPaths: Set<string>
    onToggleNode: (path: string, isOpen: boolean) => void
}

/** The vault file browser: filter box, pinned notes, section tabs and the tree. */
export function VaultSidebar(p: Props) {
    const { query, setQuery, onCollapse, pinned, pinnedExpanded, togglePinned, activePath, onNavigate, onUnpin } = p
    const { tree, treeLoading, filteredTree, section, setSection, expandedPaths, onToggleNode } = p
    return (
    <div className="flex flex-col h-full min-h-0">
        {/* Search and collapse header */}
        <div className="px-3 pt-3 pb-2 flex-shrink-0">
            <div className="flex items-center gap-2">
                <div className="relative flex-1">
                    <span
                        className="absolute left-3 top-1/2 -translate-y-1/2 text-xs pointer-events-none"
                        style={{ color: 'var(--text-dim)', opacity: 0.5 }}
                    >
                        ⌕
                    </span>
                    <input
                        type="text"
                        placeholder="Filter files..."
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        className="w-full text-xs pl-8 pr-7 py-2 rounded-xl outline-none glass-input"
                        style={{
                            color: 'var(--text-primary)',
                            fontFamily: 'var(--font-body)',
                        }}
                    />
                    {query && (
                        <button
                            onClick={() => setQuery('')}
                            className="absolute right-2 top-1/2 -translate-y-1/2 text-[11px] p-1 rounded-md transition-colors text-text-dim hover:text-text-primary bg-white/[0.06]"
                        >
                            <X className="w-3 h-3" />
                        </button>
                    )}
                </div>
                {/* Desktop sidebar collapse button */}
                <button
                    onClick={onCollapse}
                    title="Collapse sidebar"
                    className="hidden md:flex items-center justify-center p-2 rounded-xl text-text-dim hover:text-text-primary bg-white/[0.03] hover:bg-white/[0.08] border border-white/[0.05] transition-all shrink-0"
                >
                    <PanelLeftClose className="w-3.5 h-3.5" />
                </button>
            </div>
        </div>

        {pinned.length > 0 && !query && (
            <PinnedSection
                pinned={pinned}
                pinnedExpanded={pinnedExpanded}
                togglePinned={togglePinned}
                activePath={activePath}
                onNavigate={onNavigate}
                onUnpin={onUnpin}
            />
        )}

        {/* Vault Explorer Header & Section Selector */}
        <div className="flex-shrink-0 px-3 pt-2 pb-1.5">
            <div className="flex items-center justify-between gap-2 mb-2 px-1">
                <div className="flex items-center gap-1.5">
                    <Folder className="w-3.5 h-3.5 text-text-dim" />
                    <span className="text-[11px] tracking-widest uppercase font-bold text-text-primary">
                        Vault Explorer
                    </span>
                </div>
                {tree && (
                    <span className="text-[9px] text-text-dim/50">
                        {countFiles(tree)} files
                    </span>
                )}
            </div>

            {/* Section tabs as a pill switcher */}
            <div
                className="flex rounded-xl p-0.5"
                style={{ background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.04)' }}
            >
                {(['Notebooks', '_system', '_logs'] as Section[]).map((s) => (
                    <button
                        key={s}
                        onClick={() => setSection(s)}
                        className="flex-1 py-1.5 text-[9px] tracking-wider uppercase font-bold transition-all rounded-lg"
                        style={{
                            color: section === s ? 'var(--accent-blue)' : 'var(--text-dim)',
                            background: section === s ? 'rgba(0, 173, 238, 0.08)' : 'transparent',
                            boxShadow: section === s ? '0 0 12px rgba(0, 173, 238, 0.08)' : 'none',
                        }}
                    >
                        {s.replace('_', '')}
                    </button>
                ))}
            </div>
        </div>

        {/* Tree */}
        <div className="flex-1 overflow-y-auto py-1 px-1" style={{ paddingBottom: 'calc(80px + env(safe-area-inset-bottom))' }}>
            {treeLoading ? (
                <SidebarTreeSkeleton />
            ) : filteredTree && filteredTree.children && filteredTree.children.length > 0 ? (
                filteredTree.children.map((child) => (
                    <TreeNode
                        key={child.path}
                        node={child}
                        depth={0}
                        expandedPaths={expandedPaths}
                        onToggle={onToggleNode}
                        selected={activePath}
                        onFileClick={onNavigate}
                    />
                ))
            ) : filteredTree && filteredTree.type === 'file' ? (
                <TreeNode
                    node={filteredTree}
                    depth={0}
                    expandedPaths={expandedPaths}
                    onToggle={onToggleNode}
                    selected={activePath}
                    onFileClick={onNavigate}
                />
            ) : (
                <div className="text-center py-8 px-4 flex flex-col items-center gap-2">
                    <span className="text-xs text-text-dim">
                        {query ? `No files matching "${query}"` : 'No files found'}
                    </span>
                    {query && (
                        <button
                            onClick={() => setQuery('')}
                            className="text-[11px] px-2.5 py-1 rounded-lg bg-white/5 hover:bg-white/10 text-accent-blue transition-colors"
                        >
                            Clear filter
                        </button>
                    )}
                </div>
            )}
        </div>

        {/* Sidebar footer */}
        <div
            className="flex-shrink-0 px-4 py-2.5 text-[9px] flex items-center justify-between"
            style={{
                borderTop: '1px solid rgba(255,255,255,0.04)',
                color: 'var(--text-dim)',
                opacity: 0.6,
            }}
        >
            <span className="font-semibold">Agent HQ</span>
            <span>{tree ? countFiles(tree) : '...'} notes</span>
        </div>
    </div>
    )
}

interface PinnedSectionProps {
    pinned: PinnedNote[]
    pinnedExpanded: boolean
    togglePinned: () => void
    activePath: string | null
    onNavigate: () => void
    onUnpin: (path: string) => void
}

/** Pinned notes: open by default as cards, collapsed to the first three titles. */
function PinnedSection({ pinned, pinnedExpanded, togglePinned, activePath, onNavigate, onUnpin }: PinnedSectionProps) {
    return (
        <div className="flex-shrink-0 pb-2 px-3">
            <button
                onClick={togglePinned}
                className="flex items-center gap-2 mb-2 w-full text-left group py-1 px-1.5 rounded-lg hover:bg-white/[0.03] transition-all"
                title={pinnedExpanded ? 'Collapse Pinned' : 'Expand Pinned'}
            >
                <div className="flex items-center gap-1.5 min-w-0">
                    <Pin className="w-3 h-3 text-accent-blue" />
                    <span className="text-[11px] tracking-widest uppercase font-bold text-accent-blue">
                        Pinned
                    </span>
                </div>
                <div className="flex-1 h-px bg-accent-blue/15" />
                <span className="text-[9px] px-1.5 py-0.5 rounded-md bg-accent-blue/10 text-accent-blue font-bold">
                    {pinned.length}
                </span>
                <ChevronDown
                    className={`w-3.5 h-3.5 text-text-dim transition-transform duration-200 ${
                        pinnedExpanded ? 'rotate-0' : '-rotate-90'
                    }`}
                />
            </button>
            {pinnedExpanded && (
                <div className="flex flex-col gap-2 max-h-[36vh] overflow-y-auto pb-1 pr-0.5">
                    {pinned.map((n, i) => (
                        <div key={n.path} className="stagger-item" style={{ animationDelay: `${i * 40}ms` }}>
                            <PinnedCard note={n} isSelected={activePath === n.path} onClick={onNavigate} onUnpin={onUnpin} />
                        </div>
                    ))}
                </div>
            )}
            {!pinnedExpanded && (
                <div className="flex flex-col gap-1 px-1">
                    {pinned.slice(0, 3).map((n) => (
                        <Link
                            key={n.path}
                            to="/vault/$"
                            params={{ _splat: n.path }}
                            onClick={onNavigate}
                            className="text-[11px] truncate py-1 px-2 rounded-lg transition-all flex items-center gap-1.5"
                            style={{
                                color: activePath === n.path ? 'var(--accent-blue)' : 'var(--text-dim)',
                                background: activePath === n.path ? 'rgba(0, 173, 238, 0.08)' : 'transparent',
                            }}
                            onMouseEnter={(e) => { if (activePath !== n.path) e.currentTarget.style.background = 'rgba(255,255,255,0.03)' }}
                            onMouseLeave={(e) => { if (activePath !== n.path) e.currentTarget.style.background = 'transparent' }}
                        >
                            <span className="opacity-40 text-[11px]">📌</span>
                            <span className="truncate">{n.title}</span>
                        </Link>
                    ))}
                    {pinned.length > 3 && (
                        <button
                            onClick={() => togglePinned()}
                            className="text-[9px] py-0.5 px-2 text-left text-text-dim/60 hover:text-accent-blue transition-colors"
                        >
                            +{pinned.length - 3} more pinned
                        </button>
                    )}
                </div>
            )}
        </div>
    
    )
}
