import type { ReactNode } from 'react'
import type { SectionState } from '~/lib/homeSections'
import { useSharedConnection } from '~/lib/connection'
import { t, type StringKey } from '~/strings'

/** A section's own failure shows only while Connected; every other state is said by the status line and the strip. */
export const showsSectionError = (connected: boolean, kind: SectionState<unknown>['kind']) => connected && kind === 'error'

interface HomeSectionProps<T> {
  title: string
  state: SectionState<T>
  emptyText: string
  failedKey: StringKey
  onRetry: () => void
  render: (items: T[]) => ReactNode
}

const SKELETON_ROWS = 2

/** One titled group of Notes home: its count, then skeleton shapes, an empty line, its own failure or the items. */
export function HomeSection<T>({ title, state, emptyText, failedKey, onRetry, render }: HomeSectionProps<T>) {
  const connected = useSharedConnection() === 'connected'
  return (
    <section aria-label={title} data-home-section={title}>
      <p className="a-grp">
        {title}
        {state.kind === 'ready' && <em className="a-count">{state.items.length}</em>}
      </p>
      {state.kind === 'loading' && (
        <div role="status" aria-label={t('notes_loading')}>
          {Array.from({ length: SKELETON_ROWS }, (_, i) => (
            <span key={i} className="a-skel t" />
          ))}
        </div>
      )}
      {state.kind === 'empty' && <p className="a-body-sm a-muted">{emptyText}</p>}
      {showsSectionError(connected, state.kind) && (
        <div role="alert" className="a-banner">
          <span>{t(failedKey)}</span>
          <button type="button" className="a-link" onClick={onRetry}>
            {t('chat_retry')}
          </button>
        </div>
      )}
      {state.kind === 'ready' && render(state.items)}
    </section>
  )
}
