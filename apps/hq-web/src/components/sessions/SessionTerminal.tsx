import { Fragment, useEffect, useMemo, useRef } from 'react'
import { ArrowDown, Loader2 } from 'lucide-react'
import { t } from '~/strings'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { parseAnsi, plainText, spanCss } from '~/lib/ansi'
import { chooseScreenMode, screenStatusText } from '~/lib/screenStream'
import { READY_POLL_MS } from '~/lib/workbench'
import { useStickToBottom } from '../chat/useStickToBottom'
import { usePolled } from './usePolled'
import { useScreenStream } from './useScreenStream'

const SCREEN_LINES = 200

interface Props {
  sessionId: string
  /** Bumped by the parent after a send so the screen refreshes at once. */
  refreshKey: number
  active: boolean
  /** How often to re-read; the parent picks it from what the agent is doing. */
  pollMs?: number
  /** Called with the newest text so the parent can quote it. */
  onText?: (text: string) => void
  /** Named in the screen's labels. */
  helper?: string
}

const SKELETON_WIDTHS = ['w-3/4', 'w-1/2', 'w-5/6', 'w-2/3', 'w-1/3']

/** Placeholder lines shown until the first screen read answers, which can take seconds on a remote computer. */
function TerminalSkeleton({ helper }: { helper: string }) {
  return (
    <div role="status" aria-label={t('helpers_screen_loading', { helper })} data-testid="terminal-skeleton" className="flex flex-col gap-2 animate-pulse">
      {SKELETON_WIDTHS.map((w) => (
        <div key={w} className={`h-3 rounded ${w}`} style={{ background: 'var(--surface-200)' }} />
      ))}
    </div>
  )
}

/** The agent's recent text, read-only, following the newest line unless scrolled up. */
export function SessionTerminal({ sessionId, refreshKey, active, pollMs = READY_POLL_MS, onText, helper = t('helpers_the_helper') }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLPreElement>(null)
  const streamUrl = globalSessionsApi.screenStreamPath(sessionId, SCREEN_LINES)
  const stream = useScreenStream(sessionId, streamUrl, active)
  const mode = chooseScreenMode(stream.phase)
  const polled = usePolled(sessionId, () => globalSessionsApi.screen(sessionId, SCREEN_LINES, true), pollMs, active && mode === 'poll', true)
  const { refresh } = polled
  useEffect(() => {
    if (refreshKey > 0) void refresh()
  }, [refreshKey, refresh])
  // After a fallback the poller has the newer data; before it, only the stream does.
  const data = mode === 'poll' && polled.data ? polled.data : stream.data ?? polled.data
  const loading = active && !data && (mode === 'stream' || polled.loading)
  const error = mode === 'poll' ? polled.error : null
  const rawText = useMemo(() => data?.lines.join('\n') ?? '', [data])
  // The callout and the quote button need plain text, whatever the view shows.
  const rows = useMemo(() => parseAnsi(rawText), [rawText])
  const text = useMemo(() => plainText(rows), [rows])
  useEffect(() => {
    onText?.(text)
  }, [text, onText])
  const { atBottom, scrollToBottom } = useStickToBottom(scrollRef, contentRef, sessionId, text)
  const screen = { loading, error }


  return (
    <section aria-label={t('helpers_screen_aria', { helper })} className="relative flex flex-col flex-1 min-h-40">
      <div className="flex items-center gap-2 py-1.5 a-cap a-muted">
        <span>{screenStatusText(mode, stream.phase, data?.source)}</span>
        {screen.loading && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
        {screen.error && (
          <span role="alert" className="truncate min-w-0 a-warn-t">
            {screen.error}
          </span>
        )}
      </div>
      <div
        ref={scrollRef}
        role="log"
        aria-label={t('helpers_screen_aria', { helper })}
        aria-live="off"
        className="a-term flex-1 min-h-32 sm:min-h-48 overflow-auto overscroll-contain"
        tabIndex={0}
      >
        {screen.loading && !text && <TerminalSkeleton helper={helper} />}
        <pre ref={contentRef} className="whitespace-pre-wrap break-words">
          {rawText
            ? rows.map((spans, i) => (
                <Fragment key={i}>
                  {i > 0 && '\n'}
                  {spans.map((sp, j) => (
                    <span key={j} style={spanCss(sp.style)}>
                      {sp.text}
                    </span>
                  ))}
                </Fragment>
              ))
            : text || (screen.loading ? '' : 'Nothing to show yet.')}
        </pre>
      </div>
      {!atBottom && (
        <button
          type="button"
          onClick={() => scrollToBottom(true)}
          className="a-btn tonal compact absolute right-3 bottom-3"
        >
          <ArrowDown className="a-ic" aria-hidden="true" />
          {t('helpers_screen_latest')}
        </button>
      )}
    </section>
  )
}
