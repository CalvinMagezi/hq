import { Fragment, useEffect, useMemo, useRef } from 'react'
import { ArrowDown, Loader2 } from 'lucide-react'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { parseAnsi, spanCss, stripAnsi } from '~/lib/ansi'
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
}

const SKELETON_WIDTHS = ['w-3/4', 'w-1/2', 'w-5/6', 'w-2/3', 'w-1/3']

/** Placeholder lines shown until the first screen read answers, which can take seconds on a remote computer. */
function TerminalSkeleton() {
  return (
    <div role="status" aria-label="Loading what the agent is doing" data-testid="terminal-skeleton" className="flex flex-col gap-2 animate-pulse">
      {SKELETON_WIDTHS.map((w) => (
        <div key={w} className={`h-3 rounded bg-white/5 ${w}`} />
      ))}
    </div>
  )
}

/** The agent's recent text, read-only, following the newest line unless scrolled up. */
export function SessionTerminal({ sessionId, refreshKey, active, pollMs = READY_POLL_MS, onText }: Props) {
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
  const text = useMemo(() => stripAnsi(rawText), [rawText])
  const rows = useMemo(() => parseAnsi(rawText), [rawText])
  useEffect(() => {
    onText?.(text)
  }, [text, onText])
  const { atBottom, scrollToBottom } = useStickToBottom(scrollRef, contentRef, sessionId, text)
  const screen = { loading, error }


  return (
    <section aria-label="What the agent is doing" className="relative flex flex-col flex-1 min-h-40">
      <div className="flex items-center gap-2 px-3 py-1.5 text-[10px] font-mono text-neutral-500 border-b border-white/5">
        <span>{screenStatusText(mode, stream.phase, data?.source)}</span>
        {screen.loading && <Loader2 className="w-3 h-3 animate-spin" aria-label="Loading" />}
        {screen.error && (
          <span role="alert" className="truncate min-w-0" style={{ color: 'var(--accent-red)' }}>
            {screen.error}
          </span>
        )}
      </div>
      <div
        ref={scrollRef}
        role="log"
        aria-label="Agent output"
        aria-live="off"
        className="flex-1 min-h-32 sm:min-h-48 overflow-auto overscroll-contain bg-black/40 px-3 py-2"
        tabIndex={0}
      >
        {screen.loading && !text && <TerminalSkeleton />}
        <pre ref={contentRef} className="text-[11px] leading-snug font-mono text-neutral-200 whitespace-pre-wrap break-words">
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
          className="absolute right-3 bottom-3 flex items-center gap-1 h-11 sm:h-9 px-3 rounded-full bg-neutral-800 border border-white/10 text-[11px] font-mono text-neutral-200 hover:bg-neutral-700"
        >
          <ArrowDown className="w-3.5 h-3.5" />
          Latest
        </button>
      )}
    </section>
  )
}
