import { Loader2 } from 'lucide-react'
import type { HarnessSession } from '~/lib/sessionsApi'
import { tailLines } from '~/lib/workbench'
import { MoreKeys, SendError, useSend } from './SessionSendBox'

const CALLOUT_LINES = 12
const BIG_BUTTON_CLASS =
  'flex-1 flex items-center justify-center gap-2 h-12 rounded-xl border text-sm font-mono font-semibold disabled:opacity-50'

interface Props {
  session: HarnessSession
  /** The latest terminal text, so the question is readable right here. */
  screenText: string
  onSent: () => void
}

/** The common case: the agent asked permission and waits. Approve and Decline answer it. */
export function BlockedCallout({ session, screenText, onSent }: Props) {
  const { send, busy, error } = useSend(session.id, onSent)
  const tail = tailLines(screenText, CALLOUT_LINES)
  return (
    <section
      aria-label="This agent is waiting for you"
      className="border-t px-3 py-3 space-y-2"
      style={{ borderColor: 'var(--accent-amber)' }}
    >
      <h3 className="text-sm font-mono font-semibold" style={{ color: 'var(--accent-amber)' }}>
        This agent is waiting for you
      </h3>
      <p className="text-[11px] font-mono text-neutral-400">It stopped to ask before going on. Here is what it says.</p>
      {tail && (
        <pre className="max-h-40 overflow-auto overscroll-contain rounded-lg bg-black/40 px-3 py-2 text-[11px] leading-snug font-mono text-neutral-200 whitespace-pre-wrap break-words">
          {tail}
        </pre>
      )}
      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => void send({ keys: ['enter'] })}
          className={`${BIG_BUTTON_CLASS} border-emerald-400/40 bg-emerald-500/10 text-emerald-300 hover:bg-emerald-500/20`}
        >
          {busy && <Loader2 className="w-4 h-4 animate-spin" />}
          Approve
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void send({ keys: ['esc'] })}
          className={`${BIG_BUTTON_CLASS} border-rose-400/40 bg-rose-500/10 text-rose-300 hover:bg-rose-500/20`}
        >
          Decline
        </button>
      </div>
      <MoreKeys send={send} disabled={busy} />
      <SendError error={error} />
    </section>
  )
}
