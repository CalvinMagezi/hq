import { useEffect, useRef } from 'react'
import { ArrowDown, Loader2 } from 'lucide-react'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { useStickToBottom } from '../chat/useStickToBottom'
import { usePolled } from './usePolled'

const SCREEN_POLL_MS = 3_000
const SCREEN_LINES = 200

interface Props {
  sessionId: string
  /** Bumped by the parent after a send so the screen refreshes at once. */
  refreshKey: number
  active: boolean
}

const SKELETON_WIDTHS = ['w-3/4', 'w-1/2', 'w-5/6', 'w-2/3', 'w-1/3']

/** Placeholder lines shown until the first screen read answers, which can take seconds on a remote host. */
export function TerminalSkeleton() {
  return (
    <div role="status" aria-label="Loading terminal output" data-testid="terminal-skeleton" className="flex flex-col gap-2 animate-pulse">
      {SKELETON_WIDTHS.map((w) => (
        <div key={w} className={`h-3 rounded bg-white/5 ${w}`} />
      ))}
    </div>
  )
}

/** The session's recent pane text, read-only, following the newest line unless scrolled up. */
export function SessionTerminal({ sessionId, refreshKey, active }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLPreElement>(null)
  const screen = usePolled(sessionId, () => globalSessionsApi.screen(sessionId, SCREEN_LINES), SCREEN_POLL_MS, active)
  const { refresh } = screen
  useEffect(() => {
    if (refreshKey > 0) void refresh()
  }, [refreshKey, refresh])
  const text = screen.data?.lines.join('\n') ?? ''
  const { atBottom, scrollToBottom } = useStickToBottom(scrollRef, contentRef, sessionId, text)

  return (
    <section aria-label="Terminal output" className="relative flex flex-col min-h-0 flex-1">
      <div className="flex items-center gap-2 px-3 py-1.5 text-[10px] font-mono text-neutral-500 border-b border-white/5">
        <span>{screen.data?.source === 'snapshot' ? 'Last stored snapshot (agent not running)' : 'Live, refreshes every 3s'}</span>
        {screen.loading && <Loader2 className="w-3 h-3 animate-spin" aria-label="Loading" />}
        {screen.error && (
          <span role="alert" className="text-rose-400 truncate min-w-0">
            {screen.error}
          </span>
        )}
      </div>
      <div
        ref={scrollRef}
        role="log"
        aria-label="Session output"
        aria-live="off"
        className="flex-1 min-h-48 overflow-auto overscroll-contain bg-black/40 px-3 py-2"
        tabIndex={0}
      >
        {screen.loading && !text && <TerminalSkeleton />}
        <pre ref={contentRef} className="text-[11px] leading-snug font-mono text-neutral-200 whitespace-pre-wrap break-words">
          {text || (screen.loading ? '' : 'No output yet.')}
        </pre>
      </div>
      {!atBottom && (
        <button
          type="button"
          onClick={() => scrollToBottom(true)}
          className="absolute right-3 bottom-3 flex items-center gap-1 h-9 px-3 rounded-full bg-neutral-800 border border-white/10 text-[11px] font-mono text-neutral-200 hover:bg-neutral-700"
        >
          <ArrowDown className="w-3.5 h-3.5" />
          Latest
        </button>
      )}
    </section>
  )
}
