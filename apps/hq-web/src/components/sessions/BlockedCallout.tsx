import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import type { HarnessSession } from '~/lib/sessionsApi'
import { ANSWER_HOLD_MS, answerHeld, classifyBlocked, tailLines } from '~/lib/workbench'
import { MoreKeys, SendError, useSend } from './SessionSendBox'

const CALLOUT_LINES = 12
const BIG_BUTTON_CLASS =
  'flex-1 flex items-center justify-center gap-2 h-12 rounded-xl border border-white/10 bg-white/5 text-sm font-semibold hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-white/5'

const EXPLAIN = {
  approval: 'It stopped to ask before going on. Here is what it says.',
  trust: 'This agent is asking whether it may use this folder. Pick an option below.',
  other: 'It needs an answer before it can go on. Check what it says, then pick an option below.',
} as const

interface Props {
  session: HarnessSession
  /** The latest terminal text, so the question is readable right here. */
  screenText: string
  onSent: () => void | Promise<void>
}

/** The common case: the agent asked and waits. Approve and Decline appear only for a plain yes or no question. */
export function BlockedCallout({ session, screenText, onSent }: Props) {
  const { send, busy, error } = useSend(session.id, onSent)
  const tail = tailLines(screenText, CALLOUT_LINES)
  const kind = classifyBlocked(screenText)
  const [answered, setAnswered] = useState<string | null>(null)
  const held = busy || answerHeld(answered, tail)

  useEffect(() => {
    if (answered === null) return
    const timer = setTimeout(() => setAnswered(null), ANSWER_HOLD_MS)
    return () => clearTimeout(timer)
  }, [answered])

  const answerWith: typeof send = (payload) => {
    setAnswered(tail)
    return send(payload)
  }
  const answer = (key: string) => void answerWith({ keys: [key] })

  return (
    <section
      aria-label="This agent is waiting for you"
      className="sticky bottom-0 shrink-0 hq-composer border-t px-3 py-3 space-y-2"
      style={{ borderColor: 'var(--accent-amber)' }}
    >
      <h3 className="text-sm font-semibold" style={{ color: 'var(--accent-amber)' }}>
        This agent is waiting for you
      </h3>
      <p className="text-[11px] text-neutral-400">{EXPLAIN[kind]}</p>
      {tail && (
        <pre className="max-h-40 overflow-auto overscroll-contain rounded-lg bg-black/40 px-3 py-2 text-[11px] leading-snug text-neutral-200 whitespace-pre-wrap break-words">
          {tail}
        </pre>
      )}
      {kind === 'approval' && (
        <div className="flex items-center gap-2">
          <button type="button" disabled={held} onClick={() => answer('enter')} className={BIG_BUTTON_CLASS} style={{ color: 'var(--accent-green)' }}>
            {busy && <Loader2 className="w-4 h-4 animate-spin" />}
            Approve
          </button>
          <button type="button" disabled={held} onClick={() => answer('esc')} className={BIG_BUTTON_CLASS} style={{ color: 'var(--accent-red)' }}>
            Decline
          </button>
        </div>
      )}
      <MoreKeys send={answerWith} disabled={held} defaultOpen={kind !== 'approval'} />
      <SendError error={error} />
    </section>
  )
}
