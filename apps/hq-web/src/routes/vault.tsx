import { createFileRoute, Outlet, useLocation } from '@tanstack/react-router'
import { useState, useEffect, useCallback, useMemo } from 'react'
import { PanelLeftOpen, X } from 'lucide-react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { togglePinNote } from '~/lib/vaultApi'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { noteTreeQuery, pinnedNotesQuery, vaultKeys } from '~/lib/queries'
import { CopyPathMenu, type CopyMenuState } from '~/components/CopyPathMenu'
import { usePersistedState } from '~/lib/usePersistedState'
import { VaultSidebar, type Section } from '~/components/vault/VaultSidebar'
import { useSidebarResize } from '~/components/vault/useSidebarResize'

export const Route = createFileRoute('/vault')({
    component: VaultLayout,
})

const isBool = (v: unknown) => typeof v === 'boolean'

function VaultLayout() {
    const location = useLocation()

    const isVaultRoot = location.pathname === '/vault' || location.pathname === '/vault/'
    const activePath = isVaultRoot ? null : decodeURIComponent(location.pathname.replace(/^\/vault\//, ''))


    const derivedSection = useMemo<Section>(() => {
        if (activePath?.startsWith('_system/')) return '_system'
        if (activePath?.startsWith('_logs/')) return '_logs'
        return 'Notebooks'
    }, [activePath])

    const [section, setSection] = useState<Section>(derivedSection)
    const [query, setQuery] = useState('')
    const [sidebarOpen, setSidebarOpen] = useState(false)

    // Sidebar width & collapse states with persistence
    const {
        width: sidebarWidth,
        isDragging,
        onResizerMouseDown: handleResizerMouseDown,
        onResizerDoubleClick: handleDoubleClickResizer,
    } = useSidebarResize('hq-vault-sidebar-width')
    const [sidebarCollapsed, setSidebarCollapsed] = usePersistedState('hq-vault-sidebar-collapsed', false, isBool)

    const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())

    const queryClient = useQueryClient()
    const pinned = useQuery(pinnedNotesQuery).data?.notes ?? []

    const [pinnedExpanded, setPinnedExpanded] = usePersistedState('hq-vault-pinned-expanded', true, isBool)

    const [copyMenu, setCopyMenu] = useState<CopyMenuState | null>(null)
    const closeCopyMenu = useCallback(() => setCopyMenu(null), [])

    const closeSidebar = useCallback(() => {
        setSidebarOpen(false)
    }, [])

    const togglePinned = useCallback(() => setPinnedExpanded(prev => !prev), [setPinnedExpanded])

    const toggleSidebarCollapsed = useCallback(() => setSidebarCollapsed(prev => !prev), [setSidebarCollapsed])

    // Auto-close sidebar on mobile when route changes
    useEffect(() => {
        if (window.innerWidth < 768) setSidebarOpen(false)
    }, [activePath])

    useEffect(() => {
        if (derivedSection !== section && activePath) {
            setSection(derivedSection)
        }
    }, [derivedSection, activePath])

    const handleUnpin = useCallback(async (notePath: string) => {
        await togglePinNote(notePath, false)
        queryClient.invalidateQueries({ queryKey: vaultKeys.pinned })
    }, [queryClient])

    const treeQuery = useQuery(noteTreeQuery(section))
    const tree = treeQuery.data?.tree ?? null
    const treeLoading = !tree && treeQuery.isPending

    useEffect(() => {
        if (activePath) {
            const parts = activePath.split('/')
            const newExpanded = new Set(expandedPaths)
            const isSystemOrLogs = parts[0] === '_system' || parts[0] === '_logs'
            newExpanded.add(section)
            let currentPath = section === 'Notebooks' && !isSystemOrLogs ? '' : parts[0] + '/'
            for (let i = (isSystemOrLogs ? 1 : 0); i < parts.length - 1; i++) {
                currentPath += (currentPath && !currentPath.endsWith('/') ? '/' : '') + parts[i]
                newExpanded.add(currentPath)
            }
            setExpandedPaths(newExpanded)
        } else if (tree) {
            setExpandedPaths(new Set([tree.path]))
        }
    }, [activePath, section, tree])

    const toggleNode = useCallback((path: string, isOpen: boolean) => {
        setExpandedPaths(prev => {
            const next = new Set(prev)
            if (isOpen) next.add(path)
            else next.delete(path)
            return next
        })
    }, [])

    const filteredTree = useMemo(() => {
        if (!tree || !query) return tree
        const filter = (node: NoteTreeNode): NoteTreeNode | null => {
            if (node.type === 'file') return node.name.toLowerCase().includes(query.toLowerCase()) ? node : null
            const children = node.children?.map(filter).filter(Boolean) as NoteTreeNode[]
            return children?.length ? { ...node, children } : node.name.toLowerCase().includes(query.toLowerCase()) ? node : null
        }
        return filter(tree)
    }, [tree, query])

    useEffect(() => {
        if (query && filteredTree) {
            const allPaths = new Set<string>()
            const collect = (node: NoteTreeNode) => {
                if (node.type === 'dir') {
                    allPaths.add(node.path)
                    node.children?.forEach(collect)
                }
            }
            collect(filteredTree)
            setExpandedPaths(allPaths)
        }
    }, [query, filteredTree])

    const sidebar = (
        <VaultSidebar
            query={query}
            setQuery={setQuery}
            onCollapse={toggleSidebarCollapsed}
            pinned={pinned}
            pinnedExpanded={pinnedExpanded}
            togglePinned={togglePinned}
            activePath={activePath}
            onNavigate={closeSidebar}
            onUnpin={handleUnpin}
            tree={tree}
            treeLoading={treeLoading}
            filteredTree={filteredTree}
            section={section}
            setSection={setSection}
            expandedPaths={expandedPaths}
            onToggleNode={toggleNode}
        />
    )

    return (
        <div className="h-full flex flex-col overflow-hidden">
            <div className="flex-1 flex min-h-0 relative">
                {/* Mobile overlay */}
                {sidebarOpen && (
                    <div
                        className="md:hidden fixed inset-0 z-40 sidebar-overlay-animate"
                        style={{
                            background: 'rgba(0, 0, 0, 0.5)',
                            backdropFilter: 'blur(4px)',
                            WebkitBackdropFilter: 'blur(4px)',
                        }}
                        onClick={() => setSidebarOpen(false)}
                    />
                )}

                {/* Sidebar — glass panel */}
                <aside
                    className={`
                        fixed md:relative top-0 left-0 z-40
                        flex-shrink-0
                        vault-sidebar pad-safe-top
                        ${sidebarOpen ? 'w-[280px] translate-x-0 sidebar-animate-in' : 'w-[280px] -translate-x-full md:translate-x-0'}
                        ${sidebarCollapsed ? 'md:!w-0 md:!min-w-0 md:overflow-hidden md:border-r-0' : ''}
                    `}
                    style={{
                        width: sidebarCollapsed ? 0 : `${sidebarWidth}px`,
                        background: 'var(--bg-solid-surface)',
                        borderRight: sidebarCollapsed ? 'none' : '1px solid rgba(255,255,255,0.06)',
                        transition: isDragging ? 'none' : 'width 0.2s ease, transform 0.3s ease',
                    }}
                >
                    <div className="hidden md:block absolute inset-0 glass-heavy rounded-none pointer-events-none" style={{ border: 'none' }} />
                    
                    {/* Desktop resize handle on the right edge */}
                    {!sidebarCollapsed && (
                        <div
                            onMouseDown={handleResizerMouseDown}
                            onDoubleClick={handleDoubleClickResizer}
                            title="Drag to resize sidebar • Double-click to reset"
                            className={`hidden md:block sidebar-resizer ${isDragging ? 'is-dragging' : ''}`}
                        />
                    )}

                    <div className="flex flex-col h-full overflow-hidden relative z-10">
                        {/* Mobile Header */}
                        <div
                            className="md:hidden flex items-center justify-between px-4 py-3 flex-shrink-0 relative"
                            style={{ borderBottom: '1px solid rgba(255,255,255,0.05)' }}
                        >
                            <div className="flex items-center gap-2">
                                <div
                                    className="w-2 h-2 rounded-full"
                                    style={{ background: 'var(--accent-blue)', boxShadow: '0 0 8px rgba(0,173,238,0.4)' }}
                                />
                                <span className="text-xs font-mono font-bold tracking-wider uppercase" style={{ color: 'var(--text-primary)' }}>
                                    Vault Explorer
                                </span>
                            </div>
                            <button
                                onClick={() => setSidebarOpen(false)}
                                className="p-1.5 rounded-lg text-xs font-mono transition-all active:scale-95"
                                style={{
                                    color: 'var(--text-dim)',
                                    background: 'rgba(255,255,255,0.05)',
                                    border: '1px solid rgba(255,255,255,0.08)',
                                }}
                            >
                                <X className="w-3.5 h-3.5" />
                            </button>
                        </div>
                        {sidebar}
                    </div>
                </aside>

                {/* Content Outlet */}
                <main
                    className="flex-1 overflow-y-auto overflow-x-hidden min-w-0 flex flex-col relative"
                    style={{ background: 'transparent' }}
                >
                    {/* Mobile top bar to open sidebar */}
                    <div className="md:hidden flex items-center gap-2 px-3 py-2 flex-shrink-0 glass-heavy border-b border-white/5">
                        <button
                            onClick={() => setSidebarOpen(true)}
                            className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-mono"
                            style={{ color: 'var(--accent-blue)', background: 'rgba(0, 173, 238, 0.08)' }}
                        >
                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
                                <line x1="3" y1="6" x2="21" y2="6"/>
                                <line x1="3" y1="12" x2="15" y2="12"/>
                                <line x1="3" y1="18" x2="18" y2="18"/>
                            </svg>
                            <span>Files</span>
                        </button>
                    </div>

                    {/* Desktop bar when the sidebar is collapsed. In flow, so it never covers a note header. */}
                    {sidebarCollapsed && (
                        <div className="hidden md:flex items-center px-3 py-1.5 flex-shrink-0 border-b border-white/5">
                            <button
                                onClick={toggleSidebarCollapsed}
                                title="Expand sidebar"
                                className="flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs font-mono glass-card hover:border-accent-blue/40 text-text-dim hover:text-accent-blue transition-all"
                            >
                                <PanelLeftOpen className="w-3.5 h-3.5 text-accent-blue" />
                                <span>Files</span>
                            </button>
                        </div>
                    )}

                    <Outlet />
                </main>
            </div>
            <CopyPathMenu menu={copyMenu} onClose={closeCopyMenu} />
        </div>
    )
}
