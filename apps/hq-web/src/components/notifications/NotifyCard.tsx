import { useEffect, useState } from 'react'
import { hideNotifyCard, notifyCardHidden } from '~/lib/inboxCard'
import { isSharedComputer } from '~/lib/sharedComputer'
import { enablePush, getPushState } from '~/lib/webPush'
import { t } from '~/strings'

/** The one ask for notifications, shown at most once a week and never on a shared computer. */
export function NotifyCard() {
  const [show, setShow] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  useEffect(() => {
    if (isSharedComputer() || notifyCardHidden()) return
    getPushState()
      .then((s) => setShow(s === 'off' && Notification.permission === 'default'))
      .catch(() => undefined)
  }, [])
  if (!show && !note) return null
  const turnOn = async () => {
    const s = await enablePush()
    if (s === 'on') {
      setShow(false)
      setNote(t('inbox_notify_on'))
    } else if (s === 'denied') setNote(t('inbox_notify_blocked'))
  }
  return (
    <div className="a-card needs" role="status">
      <div className="a-card-head">
        <b>{t('card_notif_off_title')}</b>
      </div>
      <span className="a-muted">{note ?? t('inbox_notify_body')}</span>
      {show && (
        <div className="a-card-actions">
          <button type="button" className="a-btn secondary" onClick={() => void turnOn()}>
            {t('inbox_notify_turn_on')}
          </button>
          <button
            type="button"
            className="a-btn ghost"
            onClick={() => {
              hideNotifyCard()
              setShow(false)
            }}
          >
            {t('safe_not_now')}
          </button>
        </div>
      )}
    </div>
  )
}
