import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { Loader2, Send } from 'lucide-react'
import { ErrorText } from './ErrorText'
import { INTERRUPT_KEY, QUICK_KEYS, globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'

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
      setError(err instanceof Error && !(err instanceof TypeError) ? err.message : t('helpers_tell_failed'))
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
      <summary className="a-body-sm a-muted flex items-center min-h-11 cursor-pointer select-none">{t('helpers_more_keys')}</summary>
      <div className="a-keys pb-2" role="group" aria-label={t('helpers_keys_aria')}>
        {QUICK_KEYS.map((key) => (
          <button key={key} type="button" onClick={() => void send({ keys: [key] })} disabled={disabled} className="a-key">
            {key}
          </button>
        ))}
        <button
          type="button"
          aria-label={armed ? t('helpers_interrupt_confirm_aria') : t('helpers_interrupt_aria')}
          disabled={disabled}
          onClick={() => {
            if (!armed) return setArmed(true)
            setArmed(false)
            void send({ keys: [INTERRUPT_KEY] })
          }}
          onBlur={() => setArmed(false)}
          className={`a-key ${armed ? 'a-warn-t' : ''}`}
        >
          {armed ? t('helpers_interrupt_press_again') : INTERRUPT_KEY}
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
    <section aria-label={t('helpers_tell_this_helper_something')} className="flex flex-col">
      <div className="a-comp">
        <div className="a-comp-in">
          <div className="a-comp-field">
            <label className="a-sr" htmlFor="helper-message">
              {t('helpers_message_aria')}
            </label>
            <textarea
              id="helper-message"
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault()
                  void submitText()
                }
              }}
              disabled={busy}
              rows={1}
              placeholder={t('helpers_tell_this_helper_something')}
            />
          </div>
          <button type="button" onClick={() => void submitText()} disabled={busy || !text.trim()} className="a-send" aria-label={t('helpers_tell_send')}>
            {busy ? <Loader2 className="a-ic animate-spin" aria-hidden="true" /> : <Send className="a-ic" aria-hidden="true" />}
          </button>
        </div>
      </div>
      <div className="a-wrap">
        <MoreKeys send={send} disabled={busy} />
        <SendError error={error} />
      </div>
    </section>
  )
}
