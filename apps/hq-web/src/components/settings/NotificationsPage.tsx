import { useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { isSharedComputer } from '~/lib/sharedComputer'
import { readClock, deviceTimeZone, useClockZone } from '~/lib/clock'
import { useHubPhone, useInHub } from '~/lib/hubWindow'
import {
  DEFAULT_PUSH_PREFS,
  applyPushPrefs,
  getPushState,
  readPushPrefs,
  sendTestPush,
  type PushPrefs,
  type PushState,
} from '~/lib/webPush'
import { t, type StringKey } from '~/strings'
import { SilenceSection } from './SilenceSection'

export type NotificationsMode = PushState | 'shared' | 'hub' | null

interface ViewProps {
  mode: NotificationsMode
  prefs: PushPrefs
  busy: boolean
  host: string
  /** The outcome of the last test, kept on the page. */
  testLine: string | null
  onToggle: (key: keyof PushPrefs) => void
  onTest: () => void
  /** The window shows HQ on a paired phone, which shows its own notifications. */
  phone?: boolean
}

const BLOCKERS: Partial<Record<PushState, { title: StringKey; body: StringKey } | { line: StringKey }>> = {
  denied: { title: 'notif_blocked_title', body: 'notif_blocked_body' },
  'needs-install': { title: 'notif_install_title', body: 'notif_install_body' },
  'server-off': { line: 'notif_server_off' },
  unsupported: { line: 'notif_unsupported' },
}

function SwitchRow({ title, line, on, disabled, onToggle }: { title: string; line: string; on: boolean; disabled: boolean; onToggle: () => void }) {
  return (
    <div className="a-li">
      <div className="a-li-main">
        <b>{title}</b>
        <span>{line}</span>
      </div>
      <button type="button" role="switch" aria-checked={on} aria-label={title} disabled={disabled} onClick={onToggle} className="a-sw" />
    </div>
  )
}

/** Notifications for this browser (N1 to N5, H9); the page's logic is in `NotificationsPage`. */
export function NotificationsView({ mode, prefs, busy, host, testLine, onToggle, onTest, phone }: ViewProps) {
  if (mode === null) return null
  if (mode === 'hub') {
    return (
      <div className="a-set">
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('hub_notif_needs_title')}</b>
            <span>{t('hub_notif_needs_body')}</span>
          </div>
        </div>
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('hub_notif_updates_title')}</b>
            <span>{t(phone ? 'hub_notif_updates_body' : 'hub_notif_updates_body_here')}</span>
          </div>
        </div>
      </div>
    )
  }
  if (mode === 'shared') {
    return (
      <div className="a-card" role="status">
        <b>{t('notif_shared_title')}</b>
        <span className="a-muted">{t('notif_shared_body')}</span>
      </div>
    )
  }
  const blocker = BLOCKERS[mode as PushState]
  const on = mode === 'on'
  const locked = blocker !== undefined
  return (
    <div className="flex flex-col gap-4">
      {blocker && 'title' in blocker && (
        <div className="a-card warn" role="status">
          <b>{t(blocker.title)}</b>
          <span className="a-muted">{t(blocker.body, { host })}</span>
          {mode === 'needs-install' && (
            <Link to="/settings/version" className="a-btn secondary">
              {t('notif_install_act')}
            </Link>
          )}
        </div>
      )}
      {blocker && 'line' in blocker && <p className="a-muted a-body-sm">{t(blocker.line)}</p>}
      <h2 className="a-label">{t('notif_in_browser')}</h2>
      <div className="a-set" style={{ margin: 0 }}>
        <SwitchRow title={t('channel_needs_name')} line={t('notif_needs_desc')} on={on && prefs.needs} disabled={busy || locked} onToggle={() => onToggle('needs')} />
        <SwitchRow title={t('channel_updates_name')} line={t('notif_updates_desc')} on={on && prefs.updates} disabled={busy || locked} onToggle={() => onToggle('updates')} />
        <SwitchRow title={t('notif_details_title')} line={t('notif_web_details_off')} on={on && prefs.details} disabled={busy || !on} onToggle={() => onToggle('details')} />
      </div>
      {mode === 'denied' && <p className="a-muted a-body-sm">{t('notif_blocked_switch')}</p>}
      {on && (
        <div className="a-btns">
          <button type="button" className="a-btn secondary" disabled={busy} onClick={onTest}>
            {testLine ? t('notif_send_again') : t('notif_send_test')}
          </button>
          {testLine && <span className="a-muted a-body-sm">{testLine}</span>}
        </div>
      )}
      <div className="a-card" aria-hidden="true">
        <span className="a-cap a-muted">{t('notif_specimen_head')}</span>
        <b>{t('notif_specimen_title')}</b>
        <span>{t('notif_specimen_body')}</span>
        <span className="a-kind sm">{t('notif_specimen_open')}</span>
      </div>
      <p className="a-muted a-body-sm">{t('notif_specimen_note')}</p>
      <h2 className="a-label">{t('notif_good_title')}</h2>
      <div className="a-set" style={{ margin: 0 }}>
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('notif_each_browser_title')}</b>
            <span style={{ whiteSpace: 'normal' }}>{t('notif_each_browser_body')}</span>
          </div>
        </div>
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('notif_while_off_title')}</b>
            <span style={{ whiteSpace: 'normal' }}>{t('notif_while_off_body')}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

/** This browser's push subscription: the Needs you and Updates switches, details and a test. */
export function NotificationsPage() {
  const hub = useInHub()
  const phone = useHubPhone()
  const zone = useClockZone() || deviceTimeZone()
  const [state, setState] = useState<PushState | null>(null)
  const [prefs, setPrefs] = useState<PushPrefs>(DEFAULT_PUSH_PREFS)
  const [busy, setBusy] = useState(false)
  const [testLine, setTestLine] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    setPrefs(readPushPrefs())
    getPushState()
      .then(setState)
      .catch(() => setState('server-off'))
  }, [])

  const toggle = async (key: keyof PushPrefs) => {
    const base = state === 'on' ? prefs : { needs: false, updates: false, details: prefs.details }
    const next = { ...base, [key]: !(state === 'on' && prefs[key]) }
    setBusy(true)
    setFailed(false)
    try {
      setState(await applyPushPrefs(next))
      setPrefs(next)
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  const test = async () => {
    setBusy(true)
    try {
      const error = await sendTestPush()
      setTestLine(error === 'not_subscribed' ? t('push_not_subscribed') : error ? t('notif_test_failed') : t('notif_test_sent', { time: readClock(new Date(), zone).time }))
    } catch {
      setTestLine(t('notif_test_failed'))
    } finally {
      setBusy(false)
    }
  }

  const mode: NotificationsMode = hub ? 'hub' : isSharedComputer() ? 'shared' : state
  return (
    <>
      <NotificationsView mode={mode} phone={phone} prefs={prefs} busy={busy} host={typeof location === 'undefined' ? '' : location.host} testLine={testLine} onToggle={(k) => void toggle(k)} onTest={() => void test()} />
      {failed && <p className="a-muted a-body-sm" role="alert">{t('notif_failed')}</p>}
      <div className="mt-6">
        <SilenceSection />
      </div>
    </>
  )
}
