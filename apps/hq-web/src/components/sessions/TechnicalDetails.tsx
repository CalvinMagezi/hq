import { useRef, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { t } from '~/strings'
import { usePersistedState } from '~/lib/usePersistedState'
import { INTERRUPT_ARM_MS, interruptStep, terminalAction, INTERRUPT } from '~/lib/terminalKeys'
import { agentName } from '~/lib/workbench'
import type { HarnessSession } from '~/lib/sessionsApi'
import { SessionTerminal } from './SessionTerminal'
import { SendError, useSend } from './SessionSendBox'

type SentHandler = Parameters<typeof useSend>[1]

interface Props {
  session: HarnessSession
  refreshKey: number
  pollMs: number
  onText: (text: string) => void
  onSent: SentHandler
}

/** The helper's screen and keys. Closed until opened, so nothing is read or streamed meanwhile. */
export function TechnicalDetails({ session: s, refreshKey, pollMs, onText, onSent }: Props) {
  const [open, setOpen] = usePersistedState<boolean>(`hq.helper.screen.${s.id}`, false, (v) => typeof v === 'boolean')
  const [typing, setTyping] = useState(false)
  const [armed, setArmed] = useState(false)
  const armedAt = useRef<number | null>(null)
  const toggle = useRef<HTMLButtonElement>(null)
  const { send, busy, error } = useSend(s.id, onSent)
  const helper = agentName(s.harness)

  const disarm = () => {
    armedAt.current = null
    setArmed(false)
  }
  const onKeyDown = (e: React.KeyboardEvent) => {
    const hasSelection = (window.getSelection()?.toString() ?? '') !== ''
    const action = terminalAction(e, typing, hasSelection)
    if (action.do === 'none') return
    e.preventDefault()
    if (action.do === 'leave') return toggle.current?.focus()
    if (action.do === 'send') return void send({ keys: [action.key] })
    const step = interruptStep(armedAt.current, Date.now())
    armedAt.current = step.armedAt
    setArmed(step.armedAt !== null)
    if (step.armedAt !== null) window.setTimeout(() => armedAt.current === step.armedAt && disarm(), INTERRUPT_ARM_MS)
    if (step.send) void send({ keys: [INTERRUPT] })
  }

  return (
    <section aria-label={t('helpers_technical_details')} className="a-wrap shrink-0">
      <button
        ref={toggle}
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 min-h-11 text-left a-body-sm"
      >
        {open ? <ChevronDown className="a-ic" aria-hidden="true" /> : <ChevronRight className="a-ic" aria-hidden="true" />}
        <span className="a-body-b">{t('helpers_technical_details')}</span>
        {!open && <span className="a-muted">{t('helpers_technical_hint')}</span>}
      </button>
      {open && (
        <div onKeyDown={onKeyDown} className="flex flex-col">
          <div className="flex items-start justify-between gap-3 py-2">
            <span id="typing-label">
              <span className="a-body-b block">{t('helpers_typing_label')}</span>
              <span className="a-cap a-muted block">{t('helpers_typing_hint', { helper })}</span>
            </span>
            <button type="button" role="switch" aria-checked={typing} aria-labelledby="typing-label" onClick={() => setTyping((v) => !v)} className="a-sw" />
          </div>
          <SessionTerminal sessionId={s.id} refreshKey={refreshKey} active pollMs={pollMs} onText={onText} helper={helper} />
          {typing && armed && (
            <p role="status" className="a-cap a-warn-t py-2">
              <strong>{t('helpers_interrupt_press_again')}</strong> {t('helpers_interrupt_armed', { helper })}
            </p>
          )}
          {busy && <span className="sr-only">…</span>}
          <SendError error={error} />
        </div>
      )}
    </section>
  )
}
