import { Link } from '@tanstack/react-router'
import type { PinnedNote } from '~/lib/vaultApi'
import { relTime } from '~/lib/time'

export function PinnedCard({ note, isSelected, onClick, onUnpin }: { note: PinnedNote; isSelected: boolean; onClick?: () => void; onUnpin?: (path: string) => void }) {
    const folder = note.path.includes('/') ? note.path.split('/').slice(0, -1).join('/') : null

    return (
        <div className="relative group w-full">
            <Link
                to="/vault/$"
                params={{ _splat: note.path }}
                onClick={onClick}
                className="flex flex-col gap-1.5 px-3 py-2.5 rounded-xl transition-all w-full glass-card"
                style={{
                    borderColor: isSelected ? 'rgba(0, 173, 238, 0.35)' : undefined,
                    boxShadow: isSelected ? '0 0 20px rgba(0, 173, 238, 0.12), inset 0 1px 0 var(--glass-shine)' : undefined,
                }}
            >
                {/* Title row */}
                <div className="flex items-start justify-between gap-2">
                    <span
                        className="text-[12px] font-mono font-bold leading-snug flex-1"
                        style={{
                            color: isSelected ? 'var(--accent-blue)' : 'var(--text-primary)',
                            display: '-webkit-box',
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: 'vertical',
                            overflow: 'hidden',
                        }}
                    >
                        {note.title}
                    </span>
                    <span className="text-[9px] font-mono shrink-0 mt-0.5" style={{ color: 'var(--text-dim)' }}>
                        {relTime(note.updatedAt)}
                    </span>
                </div>

                {/* Preview text */}
                {note.preview && (
                    <p
                        className="text-[10px] leading-relaxed"
                        style={{
                            color: 'var(--text-dim)',
                            display: '-webkit-box',
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: 'vertical',
                            overflow: 'hidden',
                        }}
                    >
                        {note.preview}
                    </p>
                )}

                {/* Footer: folder path + tags */}
                <div className="flex items-center gap-1.5 flex-wrap">
                    {folder && (
                        <span
                            className="text-[9px] font-mono px-1.5 py-0.5 rounded-md"
                            style={{ background: 'rgba(255,255,255,0.04)', color: 'var(--text-dim)' }}
                        >
                            {folder}
                        </span>
                    )}
                    {note.tags?.slice(0, 3).map(tag => (
                        <span
                            key={tag}
                            className="text-[9px] font-mono px-1.5 py-0.5 rounded-md glass-tag"
                            style={{ color: 'var(--accent-blue)' }}
                        >
                            #{tag}
                        </span>
                    ))}
                </div>
            </Link>
            {onUnpin && (
                <button
                    onClick={(e) => { e.stopPropagation(); onUnpin(note.path) }}
                    title="Unpin"
                    className="absolute top-2 right-2 md:opacity-0 md:group-hover:opacity-100 transition-all w-5 h-5 flex items-center justify-center rounded-full text-[9px] font-bold hover:scale-110"
                    style={{
                        background: 'rgba(0,0,0,0.6)',
                        backdropFilter: 'blur(8px)',
                        WebkitBackdropFilter: 'blur(8px)',
                        color: 'var(--text-dim)',
                        border: '1px solid rgba(255,255,255,0.12)',
                    }}
                >
                    ✕
                </button>
            )}
        </div>
    )
}
