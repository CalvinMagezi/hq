import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { globalSessionsApi, type AnswerPayload, type DialogKind, type HarnessSession, type PendingDialog } from '~/lib/sessionsApi'
import { ANSWER_HOLD_MS, answerHeld, classifyBlocked, readDialog, tailLines, tappableOptions } from '~/lib/workbench'
import { MoreKeys, SendError, useSend } from './SessionSendBox'

const CALLOUT_LINES = 12
const BIG_BUTTON_CLASS =
  'flex-1 flex items-center justify-center gap-2 h-12 rounded-xl border border-white/10 bg-white/5 text-sm font-semibold hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-white/5'
const OPTION_CLASS =
  'w-full flex flex-col items-start gap-0.5 min-h-12 px-4 py-2 rounded-xl border border-white/10 bg-white/5 text-left text-sm font-semibold hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-white/5'

const EXPLAIN = {
  approval: 'It stopped to ask before going on. Here is what it says.',
  trust: 'This agent is asking whether it may use this folder. Pick an option below.',
  other: 'It needs an answer before it can go on. Check what it says, then pick an option below.',
} as const

const ASKS: Record<DialogKind, string> = {
  command: 'It wants to run a command.',
  edit: 'It wants to change a file.',
  path: 'It wants to reach a path outside what it may use.',
  question: 'It is asking you something.',
  trust: 'It is asking whether to trust this folder.',
  other: 'It needs an answer before it can go on.',
}

interface Props {
  session: HarnessSession
  /** The latest terminal text, so the question is readable right here. */
  screenText: string
  onSent: () => void | Promise<void>
}

/** The common case: the agent asked and waits. A dialog HQ can read gets one button per option;
 *  otherwise Approve and Decline appear only for a plain yes or no question. */
export function BlockedCallout({ session, screenText, onSent }: Props) {
  const { send, busy, error } = useSend(session.id, onSent)
  const tail = tailLines(screenText, CALLOUT_LINES)
  const kind = classifyBlocked(screenText)
  const read = readDialog(session)
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
      {read ? (
        <DialogAnswer sessionId={session.id} dialog={read.dialog} seq={read.seq} onSent={onSent} />
      ) : (
        <>
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
        </>
      )}
      <MoreKeys send={answerWith} disabled={held} defaultOpen={!read && kind !== 'approval'} />
      <SendError error={error} />
    </section>
  )
}

/** Answers by option through the host, which presses Enter only once the agent's cursor is on it. */
function useAnswer(sessionId: string, onSent: () => void | Promise<void>) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const answer = async (payload: AnswerPayload) => {
    setBusy(true)
    setError(null)
    try {
      const report = await globalSessionsApi.answer(sessionId, payload)
      if (!report.registered) setError(report.note ?? 'The agent did not take that answer. Check its screen.')
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not answer that.')
    } finally {
      // Stay busy until the refresh lands, so a second tap cannot answer the next dialog by mistake.
      await onSent()
      setBusy(false)
    }
  }
  return { answer, busy, error }
}

function DialogAnswer({ sessionId, dialog, seq, onSent }: { sessionId: string; dialog: PendingDialog; seq: number; onSent: () => void | Promise<void> }) {
  const { answer, busy, error } = useAnswer(sessionId, onSent)
  return (
    <div className="space-y-2">
      <p className="text-[11px] text-neutral-400">{ASKS[dialog.kind]}</p>
      {dialog.question && <p className="text-sm text-neutral-100 break-words">{dialog.question}</p>}
      {dialog.command && (
        <pre className="max-h-40 overflow-auto overscroll-contain rounded-lg bg-black/40 px-3 py-2 text-[11px] leading-snug text-neutral-200 whitespace-pre-wrap break-words">
          {dialog.command}
        </pre>
      )}
      {dialog.truncated && <p className="text-[11px] text-neutral-500">Shortened here. The whole text is on the agent's screen.</p>}
      <div className="flex flex-col gap-2">
        {tappableOptions(dialog).map((o) => (
          <button
            key={o.id}
            type="button"
            disabled={busy}
            onClick={() => void answer({ seq, option: o.id })}
            className={OPTION_CLASS}
            style={o.remember ? { color: 'var(--accent-amber)' } : undefined}
          >
            <span className="flex items-center gap-2">
              {busy && <Loader2 className="w-4 h-4 animate-spin" />}
              {o.label}
            </span>
            {o.remember && <span className="text-[10px] font-normal text-neutral-400">It will not ask about this again</span>}
          </button>
        ))}
        <button
          type="button"
          disabled={busy}
          onClick={() => void answer({ seq, decline: true })}
          className={OPTION_CLASS}
          style={{ color: 'var(--accent-red)' }}
        >
          Decline
        </button>
      </div>
      <SendError error={error} />
    </div>
  )
}
