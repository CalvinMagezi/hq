import { Dialog } from '~/components/ui/Dialog'
import { useEffect, useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { availableTimeZones, deviceTimeZone, readClock, setClockZone, useClockZone, zoneCity } from '~/lib/clock'
import { formatSize } from '~/lib/settingsGroups'
import { readDataUsed } from '~/lib/dataUsed'
import { fetchHello } from '~/lib/hello'
import { useInHub } from '~/lib/hubWindow'
import { reloadForNewer, useNewerHq } from '~/lib/newerHq'
import { refuseSignIn } from '~/lib/hqAuth'
import { useInstallOffer } from '~/components/InstallPrompt'
import { t } from '~/strings'

const TICK_MS = 15_000

/** Time zone: how times show in this browser, kept here only. */
export function TimeZonePage() {
  const pinned = useClockZone()
  const device = deviceTimeZone()
  const zones = useMemo(availableTimeZones, [])
  const [now, setNow] = useState<Date | null>(null)
  useEffect(() => {
    setNow(new Date())
    const id = window.setInterval(() => setNow(new Date()), TICK_MS)
    return () => window.clearInterval(id)
  }, [])
  const zone = pinned || device
  return (
    <div className="flex flex-col gap-4">
      <label className="a-field">
        <span className="a-field-label">{t('time_show_in')}</span>
        <select value={pinned} onChange={(e) => setClockZone(e.target.value)} className="a-input" style={{ colorScheme: 'dark' }}>
          <option value="">{t('time_same_browser', { city: zoneCity(device) })}</option>
          {zones.map((z) => (
            <option key={z} value={z}>
              {z.replace(/_/g, ' ')}
            </option>
          ))}
        </select>
      </label>
      <p className="a-muted a-body-sm">{t('time_kept')}</p>
      {now && <p className="a-body">{t('time_now', { time: readClock(now, zone).time, city: zoneCity(zone) })}</p>}
    </div>
  )
}

/** Install and version: the browser's install offer and the HQ version, with Reload when a newer one waits. */
export function VersionPage() {
  const { canInstall, install } = useInstallOffer()
  const newer = useNewerHq()
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return (
    <div className="flex flex-col gap-4">
      {newer && (
        <div className="a-card warn" role="status">
          <b>{t('card_newer_title')}</b>
          <span className="a-muted">{t('card_newer_body')}</span>
          <button type="button" className="a-btn" onClick={() => void reloadForNewer()}>
            {t('card_newer_act')}
          </button>
        </div>
      )}
      <div className="a-card">
        <b>{t('ver_install_title')}</b>
        <span className="a-muted">{t('ver_install_body')}</span>
        <span className="a-muted">{t('ver_install_ios')}</span>
        {canInstall && (
          <button type="button" className="a-btn secondary" onClick={() => void install()}>
            {t('ver_install_act')}
          </button>
        )}
      </div>
      <div className="a-card">
        <b>{t('ver_version_title')}</b>
        {hello && <span className="a-muted">{t('ver_hq_line', { version: hello.version })}</span>}
      </div>
    </div>
  )
}

/** Data used by HQ in this browser this month. */
export function DataUsedPage() {
  const hub = useInHub()
  const [bytes, setBytes] = useState<number | null>(null)
  useEffect(() => setBytes(readDataUsed()), [])
  return (
    <div className="flex flex-col gap-4">
      <div className="a-card">
        <b>{t('data_month_title')}</b>
        <span className="a-muted">{hub ? t('data_used_row_window') : bytes === null ? '' : formatSize(bytes)}</span>
      </div>
      <div className="a-card">
        <b>{t('data_idle_title')}</b>
        <span className="a-muted">{t('data_idle_body')}</span>
      </div>
      <p className="a-muted a-body-sm">{t('data_note')}</p>
    </div>
  )
}

/** The stacked dialog behind Sign out this browser: it keeps nothing of HQ. */
export function SignOutDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog
      kind="red"
      title={t('signout_browser_title')}
      body={t('signout_browser_body')}
      keepLabel={t('sign_out_device_keep')}
      actLabel={t('row_sign_out_browser')}
      onKeep={onClose}
      onAct={() => {
        onClose()
        refuseSignIn()
      }}
    />
  )
}
