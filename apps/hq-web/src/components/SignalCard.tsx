import { Link } from '@tanstack/react-router'
import type { RecentFile } from '~/lib/vaultApi'
import { relTime } from '~/lib/time'

function badgeColor(kind: string) {
  switch (kind) {
    case 'review': return 'var(--accent-violet)'
    case 'work': return 'var(--accent-green)'
    case 'activity': return 'var(--text-dim)'
    default: return 'var(--accent-blue)'
  }
}

export function SignalCard({ note, lane = 'note' }: { note: RecentFile; lane?: 'note' | 'work' | 'review' | 'activity' }) {
  const folder = note.path.split('/').slice(0, -1).join('/')
  const badges = [
    note.status,
    note.domain,
    note.primaryAgent,
    note.updatedBy ? `by ${note.updatedBy}` : undefined,
  ].filter(Boolean) as string[]

  return (
    <Link
      to="/vault/$"
      params={{ _splat: note.path }}
      className="block glass-card rounded-xl p-3.5 group"
    >
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <div className="min-w-0">
          <div className="text-[12px] font-mono font-bold truncate" style={{ color: 'var(--text-primary)' }}>
            {note.title}
          </div>
          <div className="text-[9px] font-mono mt-1 truncate" style={{ color: 'var(--text-dim)', opacity: 0.55 }}>
            {folder}
          </div>
        </div>
        <span className="text-[9px] font-mono flex-shrink-0 mt-0.5" style={{ color: 'var(--text-dim)' }}>
          {relTime(note.mtime)}
        </span>
      </div>

      {note.preview && (
        <p className="text-[11px] font-mono line-clamp-2 leading-relaxed" style={{ color: 'var(--text-dim)' }}>
          {note.preview}
        </p>
      )}

      {(badges.length > 0 || note.tags.length > 0) && (
        <div className="flex flex-wrap gap-1 mt-2">
          {badges.slice(0, 4).map((badge) => (
            <span
              key={badge}
              className="text-[8px] px-1.5 py-0.5 rounded-md font-mono glass-tag"
              style={{ color: badgeColor(lane) }}
            >
              {badge}
            </span>
          ))}
          {note.tags.slice(0, Math.max(0, 4 - badges.length)).map((tag) => (
            <span key={tag} className="text-[8px] px-1.5 py-0.5 rounded-md font-mono glass-tag" style={{ color: 'var(--text-dim)' }}>
              #{tag}
            </span>
          ))}
        </div>
      )}
    </Link>
  )
}
