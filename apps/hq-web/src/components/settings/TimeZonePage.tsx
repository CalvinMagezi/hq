import { useEffect, useMemo, useState } from 'react'
import { availableTimeZones, deviceTimeZone, readClock, setClockZone, useClockZone, zoneCity } from '~/lib/clock'
import { useHqZone, zonesDiffer } from '~/lib/time'
import { t } from '~/strings'

const TICK_MS = 15_000

/** Time zone: how times show in this browser, HQ's own zone, and when the two are named. */
export function TimeZonePage() {
  const pinned = useClockZone()
  const device = deviceTimeZone()
  const hq = useHqZone()
  const zones = useMemo(availableTimeZones, [])
  const [now, setNow] = useState<Date | null>(null)
  useEffect(() => {
    setNow(new Date())
    const id = window.setInterval(() => setNow(new Date()), TICK_MS)
    return () => window.clearInterval(id)
  }, [])
  const zone = pinned || device
  const differ = zonesDiffer(hq, zone, now?.getTime())
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
      {hq && (
        <div className="a-card">
          <b>{t('tz_hq_title')}</b>
          <span className="a-muted">{t('tz_hq_body')}</span>
          <span>{zoneCity(hq)}</span>
        </div>
      )}
      {now && (
        <p className="a-body">
          {hq && differ
            ? t('tz_now_two', { time: readClock(now, zone).time, city: zoneCity(zone), hqTime: readClock(now, hq).time, hqCity: zoneCity(hq) })
            : t('time_now', { time: readClock(now, zone).time, city: zoneCity(zone) })}
        </p>
      )}
      {hq && differ && (
        <div className="a-card" role="status">
          <b>{t('tz_differ_title')}</b>
          <span className="a-muted">{t('tz_differ_body', { city: zoneCity(zone) })}</span>
        </div>
      )}
    </div>
  )
}
