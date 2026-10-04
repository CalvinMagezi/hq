import { useEffect, useState } from 'react'
import { Loader2, Send } from 'lucide-react'
import { INTERRUPT_KEY, QUICK_KEYS, globalSessionsApi, isBlocked, type HarnessSession } from '~/lib/sessionsApi'

interface Props {
  session: HarnessSession
  enabled: boolean
  onSent: () => void
}

const KEY_BUTTON_CLASS =
  'h-11 sm:h-9 min-w-11 sm:min-w-9 px-2.5 rounded border border-white/10 text-[11px] font-mono text-neutral-300 hover:text-white hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent'
const INTERRUPT_BUTTON_CLASS =
  'h-11 sm:h-9 px-2.5 rounded border border-rose-400/40 text-[11px] font-mono text-rose-300 hover:text-rose-200 hover:bg-rose-500/10 disabled:opacity-40 disabled:hover:bg-transparent'
/** How long the interrupt button stays armed before it falls back to unarmed. */
const INTERRUPT_CONFIRM_MS = 4_000

/** A prompt box and the few keys that answer a dialog. The server refuses text while the agent is blocked. */
export function SessionSendBox({ session, enabled, onSent }: Props) {
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [armed, setArmed] = useState(false)
  const blocked = isBlocked(session)

  useEffect(() => {
    if (!armed) return
    const timer = setTimeout(() => setArmed(false), INTERRUPT_CONFIRM_MS)
    return () => clearTimeout(timer)
  }, [armed])

  const send = async (payload: Parameters<typeof globalSessionsApi.send>[1]) => {
    setBusy(true)
    setError(null)
    try {
      await globalSessionsApi.send(session.id, payload)
      if (payload.text !== undefined) setText('')
      onSent()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Send failed')
      // A refused prompt usually means a dialog opened; show the screen it is waiting on.
      onSent()
    } finally {
      setBusy(false)
    }
  }

  const submitText = () => {
    if (text.trim()) void send({ text })
  }

  return (
    <section aria-label="Send to session" className="border-t border-white/10 px-3 py-2 space-y-2">
      {blocked && (
        <p className="text-[11px] font-mono" style={{ color: 'var(--accent-amber)' }}>
          Blocked at a dialog. Text is refused until it is answered; use the keys below.
        </p>
      )}
      <div className="flex items-end gap-2">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault()
              submitText()
            }
          }}
          disabled={!enabled || busy || blocked}
          rows={2}
          placeholder={enabled ? 'Prompt for the agent (Ctrl+Enter to send)' : 'The session is not running'}
          aria-label="Prompt for the agent"
          className="flex-1 min-w-0 px-3 py-2 rounded-xl text-xs font-mono text-neutral-200 bg-black/30 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400 disabled:opacity-50 resize-y"
        />
        <button
          type="button"
          onClick={submitText}
          disabled={!enabled || busy || blocked || !text.trim()}
          className="flex items-center justify-center gap-1.5 h-11 sm:h-9 px-3 rounded-xl border border-white/10 text-xs font-mono text-neutral-200 hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent shrink-0"
        >
          {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Send className="w-3.5 h-3.5" />}
          Send
        </button>
      </div>
      <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Press a key">
        <span className="text-[10px] font-mono text-neutral-500 mr-1">Keys</span>
        {QUICK_KEYS.map((key) => (
          <button key={key} type="button" onClick={() => void send({ keys: [key] })} disabled={!enabled || busy} className={KEY_BUTTON_CLASS}>
            {key}
          </button>
        ))}
        <span className="flex-1" />
        <button
          type="button"
          aria-label={armed ? 'Confirm interrupt (Ctrl+C)' : 'Interrupt (Ctrl+C)'}
          disabled={!enabled || busy}
          onClick={() => {
            if (!armed) return setArmed(true)
            setArmed(false)
            void send({ keys: [INTERRUPT_KEY] })
          }}
          onBlur={() => setArmed(false)}
          className={INTERRUPT_BUTTON_CLASS}
        >
          {armed ? 'tap again to interrupt' : INTERRUPT_KEY}
        </button>
      </div>
      {error && (
        <p role="alert" className="text-[11px] font-mono text-rose-400 whitespace-pre-wrap break-words max-h-24 overflow-auto">
          {error}
        </p>
      )}
    </section>
  )
}
