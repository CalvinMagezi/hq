import { useState } from 'react'
import { Dialog } from '~/components/ui/Dialog'
import { t } from '~/strings'

/** "14:32 today", or "14:32 on 3 Nov" for an older item, in this browser's clock. */
export function signInTime(createdAt: string, now: Date = new Date()): string {
  const at = new Date(createdAt)
  const time = at.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
  if (at.toDateString() === now.toDateString()) return t('new_signin_today', { time })
  return t('new_signin_day', { time, date: at.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' }) })
}

/** The two sentences of a new sign-in card, from the device's name and the item's time. */
export function SignInBody({ device, createdAt }: { device: string | null; createdAt: string }) {
  return (
    <>
      <p className="a-body-sm">{t('new_signin_body', { device: device ?? t('new_signin_unknown'), time: signInTime(createdAt) })}</p>
      <p className="a-body-sm a-muted">{t('new_signin_advice')}</p>
    </>
  )
}

interface ActionsProps {
  device: string | null
  busy: boolean
  onItWasMe: () => void
  /** Signs the device out on HQ; rejects with the words to show. */
  onSignOut(): Promise<void>
}

/** It was me (secondary) and Sign it out (red, and it asks first in the shared dialog). */
export function SignInActions({ device, busy, onItWasMe, onSignOut }: ActionsProps) {
  const [asking, setAsking] = useState(false)
  const [working, setWorking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const name = device ?? t('new_signin_unknown')
  const act = async () => {
    setWorking(true)
    setError(null)
    try {
      await onSignOut()
      setAsking(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : t('device_out_failed'))
    } finally {
      setWorking(false)
    }
  }
  return (
    <div className="a-card-actions">
      <button type="button" className="a-btn secondary" disabled={busy} onClick={onItWasMe}>
        {t('it_was_me')}
      </button>
      <button type="button" className="a-btn danger" disabled={busy} onClick={() => setAsking(true)}>
        {t('sign_it_out')}
      </button>
      {asking && (
        <Dialog
          kind="red"
          title={t('sign_out_device_title', { device: name })}
          body={t('sign_out_device_body')}
          keepLabel={t('sign_out_device_keep')}
          actLabel={t('sign_out_device_act')}
          busyLabel={t('confirm_busy_signing_out')}
          busy={working}
          error={error}
          onKeep={() => setAsking(false)}
          onAct={() => void act()}
        />
      )}
    </div>
  )
}
