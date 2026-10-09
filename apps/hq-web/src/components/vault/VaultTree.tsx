import { Link } from '@tanstack/react-router'
import { ChevronRight } from 'lucide-react'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { fileIcon } from './fileIcon'

export function SidebarTreeSkeleton() {
    return (
        <div className="flex flex-col gap-2 px-3 py-3">
            {[
                { width: '65%', depth: 0 },
                { width: '45%', depth: 1 },
                { width: '80%', depth: 1 },
                { width: '55%', depth: 0 },
                { width: '70%', depth: 1 },
                { width: '40%', depth: 2 },
                { width: '60%', depth: 0 },
            ].map((item, idx) => (
                <div
                    key={idx}
                    className="flex items-center gap-2 py-1.5"
                    style={{ paddingLeft: `${item.depth * 14}px` }}
                >
                    <div className="w-3.5 h-3.5 rounded skeleton-shimmer shrink-0" />
                    <div
                        className="h-3 rounded skeleton-shimmer"
                        style={{ width: item.width }}
                    />
                </div>
            ))}
        </div>
    )
}

export function TreeNode({
    node,
    depth = 0,
    selected,
    expandedPaths,
    onToggle,
    onFileClick,
}: {
    node: NoteTreeNode
    depth?: number
    selected: string | null
    expandedPaths: Set<string>
    onToggle: (path: string, isOpen: boolean) => void
    onFileClick: () => void
}) {
    const isOpen = expandedPaths.has(node.path)

    if (node.type === 'dir') {
        return (
            <div>
                <button
                    onClick={() => onToggle(node.path, !isOpen)}
                    className="w-full text-left py-1.5 px-2 text-xs font-medium flex items-center gap-1.5 rounded-lg transition-all mx-0.5 group"
                    style={{
                        paddingLeft: `${8 + depth * 14}px`,
                        color: 'var(--text-dim)',
                        background: 'transparent',
                    }}
                    onMouseEnter={(e) => {
                        e.currentTarget.style.background = 'rgba(255,255,255,0.03)'
                        e.currentTarget.style.color = 'var(--text-primary)'
                    }}
                    onMouseLeave={(e) => {
                        e.currentTarget.style.background = 'transparent'
                        e.currentTarget.style.color = 'var(--text-dim)'
                    }}
                >
                    <ChevronRight
                        className={`w-3 h-3 text-text-dim/50 group-hover:text-text-dim transition-transform duration-150 shrink-0 ${
                            isOpen ? 'rotate-90 text-accent-blue' : 'rotate-0'
                        }`}
                    />
                    <span className="shrink-0 opacity-70 group-hover:opacity-100 text-[12px]">
                        {isOpen ? '📂' : '📁'}
                    </span>
                    <span className="truncate">{node.name}</span>
                    {node.children && node.children.length > 0 && (
                        <span className="text-[9px] px-1 py-0.2 rounded bg-white/[0.04] text-text-dim/60 ml-auto mr-1 ">
                            {node.children.length}
                        </span>
                    )}
                </button>
                {isOpen && (
                    <div className="relative">
                        {/* Indent guide line */}
                        <div
                            className="absolute top-0 bottom-0 w-px"
                            style={{
                                left: `${14 + depth * 14}px`,
                                background: 'rgba(255,255,255,0.04)',
                            }}
                        />
                        {node.children?.map((c) => (
                            <TreeNode
                                key={c.path}
                                node={c}
                                depth={depth + 1}
                                expandedPaths={expandedPaths}
                                onToggle={onToggle}
                                selected={selected}
                                onFileClick={onFileClick}
                            />
                        ))}
                    </div>
                )}
            </div>
        )
    }

    const isSelected = selected === node.path
    return (
        <Link
            to="/vault/$"
            params={{ _splat: node.path }}
            onClick={onFileClick}
            className="w-full text-left py-1.5 px-2 text-xs flex items-center gap-1.5 rounded-lg transition-all mx-0.5 group tree-node-transition"
            style={{
                paddingLeft: `${14 + depth * 14}px`,
                background: isSelected ? 'rgba(0, 173, 238, 0.08)' : 'transparent',
                color: isSelected ? 'var(--accent-blue)' : 'var(--text-dim)',
                borderLeft: isSelected ? '2px solid var(--accent-blue)' : '2px solid transparent',
            }}
            onMouseEnter={(e) => {
                if (!isSelected) {
                    e.currentTarget.style.background = 'rgba(255,255,255,0.03)'
                    e.currentTarget.style.color = 'var(--text-primary)'
                }
            }}
            onMouseLeave={(e) => {
                if (!isSelected) {
                    e.currentTarget.style.background = 'transparent'
                    e.currentTarget.style.color = 'var(--text-dim)'
                }
            }}
        >
            <span className="opacity-50 group-hover:opacity-80 flex-shrink-0 text-[11px]">{fileIcon(node.name)}</span>
            <span className="truncate">{node.name.replace(/\.md$/, '')}</span>
        </Link>
    )
}

export function countFiles(node: NoteTreeNode): number {
    if (node.type === 'file') return 1
    return node.children?.reduce((acc, c) => acc + countFiles(c), 0) ?? 0
}
