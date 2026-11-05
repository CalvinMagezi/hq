import { useEffect, useRef, useState, type RefObject } from 'react'
import type { UseConnectionResult } from '~/lib/connection'
import { relTime } from '~/lib/time'
import { t } from '~/strings'

export interface ConnectionSheetProps {
  open: boolean
  onClose: () => void
  triggerRef?: RefObject<HTMLElement | null>
  connection: UseConnectionResult
}

function formatLastAnswer(timestamp: number | null): string {
  if (timestamp === null) return t('sheet_never')
  const date = new Date(timestamp)
  const timeStr = new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date)
  const ago = relTime(timestamp)
  return ago ? `${timeStr}, ${ago}` : timeStr
}

const OVERLAY_STYLE = { position: 'fixed', inset: 0, zIndex: 60, background: 'transparent' } as const

export function ConnectionSheet({ open, onClose, triggerRef, connection }: ConnectionSheetProps) {  const sheetRef = useRef<HTMLDivElement>(null)
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    if (!open) return undefined
    const id = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [open])

  useEffect(() => {
    if (!open) return undefined
    const prev = document.activeElement as HTMLElement | null
    sheetRef.current?.focus()

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        onClose()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      if (triggerRef?.current) {
        triggerRef.current.focus()
      } else if (prev && typeof prev.focus === 'function') {
        prev.focus()
      }
    }
  }, [open, onClose, triggerRef])

  if (!open) return null

  const address = typeof window !== 'undefined' ? window.location.host : 'hq.example.ts.net'
  const lastAnswerText = formatLastAnswer(connection.lastHelloAnsweredAt)
  const remainingSeconds = connection.nextTryMs !== null
    ? Math.max(0, Math.ceil((connection.nextTryMs - now) / 1000))
    : 20
  const countdownText = t('sheet_seconds_remaining', { seconds: remainingSeconds })

  // The design system lays out against its `.a-app` container, which today's shell does not have yet:
  // the sheet brings its own, fixed over the whole window and above every pane.
  return (
    <div className="a-app" style={OVERLAY_STYLE}>
      <div className="a-scrim" onClick={onClose} />
      <div
        ref={sheetRef}
        tabIndex={-1}
        className="a-sheet side"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cs-t"
      >
        <div className="a-handle" />
        <div className="a-sheet-head">
          <h2 className="a-h2" id="cs-t">{connection.words}</h2>
          <button className="a-ib" type="button" aria-label={t('close')} onClick={onClose}>
            <svg className="a-ic" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>
        <div className="a-sheet-body">
          {connection.state === 'unreachable' && (
            <>
              <p className="a-body-sm a-muted">
                {t('sheet_unreachable_body', { address })}
              </p>
              <div>
                <div className="a-drow" style={{ padding: 0 }}>
                  <span>{t('sheet_last_answer')}</span>
                  <span>{lastAnswerText}</span>
                </div>
                <div className="a-drow" style={{ padding: 0 }}>
                  <span>{t('sheet_trying_again')}</span>
                  <span>{countdownText}</span>
                </div>
                <div className="a-drow" style={{ padding: 0 }}>
                  <span>{t('sheet_this_browser')}</span>
                  <span>{connection.online ? t('sheet_online') : t('sheet_offline')}</span>
                </div>
              </div>
              <p className="a-body-sm">{t('sheet_saved_copy_line')}</p>
              <button className="a-btn block" type="button" onClick={connection.tryNow}>
                {t('sheet_try_again_now')}
              </button>
              <a className="a-btn quiet block" href="#help">
                {t('sheet_what_else_check')}
              </a>
            </>
          )}

          {connection.state === 'starting' && (
            <>
              <p className="a-body-sm a-muted">{t('sheet_starting_body')}</p>
              {connection.hello?.starting && (
                <p className="a-body-sm">
                  {t('sheet_starting_step', {
                    step: connection.hello.starting.step,
                    steps: connection.hello.starting.steps,
                    step_name: connection.hello.starting.step_name,
                  })}
                </p>
              )}
            </>
          )}

          {connection.state === 'connected' && (
            <>
              <p className="a-body-sm a-muted">{t('sheet_connected_body')}</p>
              <div className="a-drow" style={{ padding: 0 }}>
                <span>{t('sheet_address_label')}</span>
                <span>{address}</span>
              </div>
            </>
          )}

          {connection.state === 'offline' && (
            <p className="a-body-sm a-muted">{t('fail_send_message_offline')}</p>
          )}

          {connection.state === 'signed_out' && (
            <p className="a-body-sm a-muted">{t('reason_signed_out')}</p>
          )}

          {connection.state === 'reconnecting' && (
            <p className="a-body-sm a-muted">{t('state_reconnecting')}</p>
          )}

          {connection.state === 'ai_down' && (
            <p className="a-body-sm a-muted">{t('fail_finish_reply_ai_down')}</p>
          )}
        </div>
      </div>
    </div>
  )
}
