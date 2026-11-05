import type { ReactNode } from 'react'
import type { SectionState } from '~/lib/homeSections'
import { useSharedConnection } from '~/lib/connection'
import { t, type StringKey } from '~/strings'

const FAILED_LOAD: Record<string, StringKey> = {
  Pinned: 'fail_load_pinned_no_reply',
  'Recent Project Notes': 'fail_load_recent_no_reply',
  'In Progress Tasks': 'fail_load_tasks_no_reply',
}

/** A section's own failure shows only while Connected; every other state is said by the status line and the strip. */
export const showsSectionError = (connected: boolean, kind: SectionState<unknown>['kind']) => connected && kind === 'error'

interface HomeSectionProps<T> {
  title: string
  state: SectionState<T>
  emptyText: string
  onRetry: () => void
  render: (items: T[]) => ReactNode
  accent?: string
}

const SKELETON_ROWS = 2

export function HomeSection<T>({ title, state, emptyText, onRetry, render, accent }: HomeSectionProps<T>) {
  const connected = useSharedConnection() === 'connected'
  return (
    <section className="mb-10" aria-label={title} data-home-section={title}>
      <div className="flex items-center gap-2 mb-4">
        <span className="text-xs tracking-widest uppercase font-bold" style={{ color: accent ?? 'var(--text-dim)' }}>
          {title}
        </span>
        <div className="flex-1 h-px bg-white/5" />
        {state.kind === 'ready' && (
          <span className="text-xs px-1.5 py-0.5 rounded-md bg-white/5 font-bold" style={{ color: 'var(--text-dim)' }}>
            {state.items.length}
          </span>
        )}
      </div>
      {state.kind === 'loading' && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3" role="status" aria-label={`Loading ${title}`}>
          {Array.from({ length: SKELETON_ROWS }, (_, i) => (
            <div key={i} className="glass-card rounded-xl h-16 animate-pulse" />
          ))}
        </div>
      )}
      {state.kind === 'empty' && (
        <div className="glass-card rounded-xl px-4 py-5 text-xs " style={{ color: 'var(--text-dim)' }}>
          {emptyText}
        </div>
      )}
      {showsSectionError(connected, state.kind) && (
        <div role="alert" className="glass-card rounded-xl px-4 py-4 flex items-center justify-between gap-3 text-xs " style={{ color: 'var(--text-dim)' }}>
          <span className="min-w-0 break-words">{t(FAILED_LOAD[title] ?? 'fail_load_recent_no_reply')}</span>
          <button type="button" onClick={onRetry} className="shrink-0 px-3 py-1.5 rounded-lg bg-white/5 font-bold" style={{ color: 'var(--text-primary)' }}>
            {t('chat_retry')}
          </button>
        </div>
      )}
      {state.kind === 'ready' && render(state.items)}
    </section>
  )
}
