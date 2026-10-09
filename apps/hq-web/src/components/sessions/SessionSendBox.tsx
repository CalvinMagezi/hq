import { useEffect, useState } from 'react'
import { Loader2, Send } from 'lucide-react'
import { ErrorText } from './ErrorText'
import { INTERRUPT_KEY, QUICK_KEYS, globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'

const KEY_BUTTON_CLASS =
  'h-11 sm:h-9 min-w-11 sm:min-w-9 px-2.5 rounded border border-white/10 text-[11px] font-mono text-neutral-300 hover:text-white hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent'
const INTERRUPT_BUTTON_CLASS =
  'h-11 sm:h-9 px-2.5 rounded border border-white/10 text-[11px] font-mono hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent'
/** How long the interrupt button stays armed before it falls back to unarmed. */
const INTERRUPT_CONFIRM_MS = 4_000

type SendPayload = Parameters<typeof globalSessionsApi.send>[1]

/** Sends text or keys to one agent, tracking busy and error state for whoever shows the controls. */
export function useSend(sessionId: string, onSent: () => void | Promise<void>) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const send = async (payload: SendPayload): Promise<boolean> => {
    setBusy(true)
    setError(null)
    try {
      await globalSessionsApi.send(sessionId, payload)
      // Stay busy until the refresh lands, so a second tap cannot answer a stale screen.
      await onSent()
      return true
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not send that.')
      // A refused prompt usually means a dialog opened; show the screen it is waiting on.
      await onSent()
      return false
    } finally {
      setBusy(false)
    }
  }
  return { send, busy, error }
}

export function SendError({ error }: { error: string | null }) {
  return error ? <ErrorText className="whitespace-pre-wrap max-h-24 overflow-auto">{error}</ErrorText> : null
}

interface KeysProps {
  send: (payload: SendPayload) => Promise<boolean>
  disabled: boolean
  /** Opened at once when the buttons are the only way to answer. */
  defaultOpen?: boolean
}

/** The raw keys for the rare dialog that Approve and Decline do not answer, plus a two-tap interrupt. */
export function MoreKeys({ send, disabled, defaultOpen }: KeysProps) {
  const [armed, setArmed] = useState(false)
  useEffect(() => {
    if (!armed) return
    const timer = setTimeout(() => setArmed(false), INTERRUPT_CONFIRM_MS)
    return () => clearTimeout(timer)
  }, [armed])
  return (
    <details open={defaultOpen}>
      <summary className="flex items-center min-h-11 cursor-pointer text-[11px] font-mono text-neutral-400 hover:text-white select-none">More keys</summary>
      <div className="flex flex-wrap items-center gap-1.5 pb-1" role="group" aria-label="Press a key">
        {QUICK_KEYS.map((key) => (
          <button key={key} type="button" onClick={() => void send({ keys: [key] })} disabled={disabled} className={KEY_BUTTON_CLASS}>
            {key}
          </button>
        ))}
        <span className="flex-1" />
        <button
          type="button"
          aria-label={armed ? 'Confirm stop what it is doing (Ctrl+C)' : 'Stop what it is doing (Ctrl+C)'}
          disabled={disabled}
          onClick={() => {
            if (!armed) return setArmed(true)
            setArmed(false)
            void send({ keys: [INTERRUPT_KEY] })
          }}
          onBlur={() => setArmed(false)}
          className={INTERRUPT_BUTTON_CLASS}
          style={{ color: 'var(--accent-red)' }}
        >
          {armed ? 'tap again to interrupt' : INTERRUPT_KEY}
        </button>
      </div>
    </details>
  )
}

interface Props {
  session: HarnessSession
  onSent: () => void | Promise<void>
}

/** A message box for an agent that is ready, with the raw keys tucked away. */
export function SessionSendBox({ session, onSent }: Props) {
  const [text, setText] = useState('')
  const { send, busy, error } = useSend(session.id, onSent)

  const submitText = async () => {
    if (text.trim() && (await send({ text }))) setText('')
  }

  return (
    <section aria-label="Message the agent" className="border-t border-white/10 px-3 py-2 space-y-1">
      <div className="flex items-end gap-2">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault()
              void submitText()
            }
          }}
          disabled={busy}
          rows={2}
          placeholder="Tell the agent what to do next"
          aria-label="Message for the agent"
          className="flex-1 min-w-0 px-3 py-2 rounded-xl text-xs font-mono text-neutral-200 bg-black/30 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400 disabled:opacity-50 resize-y"
        />
        <button
          type="button"
          onClick={() => void submitText()}
          disabled={busy || !text.trim()}
          className="flex items-center justify-center gap-1.5 h-11 sm:h-9 px-3 rounded-xl border border-white/10 text-xs font-mono text-neutral-200 hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent shrink-0"
        >
          {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Send className="w-3.5 h-3.5" />}
          Send
        </button>
      </div>
      <MoreKeys send={send} disabled={busy} />
      <SendError error={error} />
    </section>
  )
}
