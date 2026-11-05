import { useCallback, useEffect, useRef, useState } from 'react'
import { Dialog } from '~/components/ui/Dialog'
import { t } from '~/strings'
import { hasWebToken, saveWebToken } from '~/lib/hqAuth'
import { toastAlert } from '~/lib/toast'
import { AlertTriangle, MoreHorizontal } from 'lucide-react'
import {
  codeLink,
  formatCountdown,
  keepDevice,
  lastUsedText,
  listDevices,
  requestDeviceCode,
  secondsLeft,
  signOutDevice,
  signOutEverywhere,
  type DeviceList,
  type SignedInDevice,
} from '~/lib/deviceSignIn'

const TICK_MS = 1000
const MS_PER_SECOND = 1000
const QR_SIZE_PX = 200
const QR_MARGIN_MODULES = 2

interface Issued {
  code: string
  deadlineMs: number
}

function errorText(e: unknown, fallback: string): string {
  return e instanceof Error ? e.message : fallback
}

function ErrorLine({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <p className="text-xs" role="alert" style={{ color: 'var(--warn)' }}>
      {message}
    </p>
  )
}

/** Draws the sign-in link as a QR code; the library loads only when someone asks for one. */
function LoginQr({ link }: { link: string }) {
  const canvas = useRef<HTMLCanvasElement>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let live = true
    import('qrcode')
      .then((qr) => {
        if (live && canvas.current) return qr.toCanvas(canvas.current, link, { width: QR_SIZE_PX, margin: QR_MARGIN_MODULES })
      })
      .catch(() => live && setFailed(true))
    return () => {
      live = false
    }
  }, [link])
  if (failed) return <p className="text-xs text-neutral-400">{t('code_qr_failed')}</p>
  return <canvas ref={canvas} width={QR_SIZE_PX} height={QR_SIZE_PX} aria-label={t('code_qr_label')} className="rounded-lg" />
}

/** A one-time code and its QR code, for signing in a new device. */
export function SignInAnother() {
  const [issued, setIssued] = useState<Issued | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const left = issued ? secondsLeft(issued.deadlineMs, now) : 0
  const ticking = left > 0

  useEffect(() => {
    if (!ticking) return
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS)
    return () => window.clearInterval(timer)
  }, [ticking])

  const start = async () => {
    setBusy(true)
    setError(null)
    try {
      const fresh = await requestDeviceCode()
      // Counted from receipt, so a clock that is off on either side does not matter.
      setIssued({ code: fresh.code, deadlineMs: Date.now() + fresh.expires_in * MS_PER_SECOND })
      setNow(Date.now())
    } catch (e) {
      setError(errorText(e, t('code_get_failed')))
    } finally {
      setBusy(false)
    }
  }

  const showing = issued !== null && left > 0
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold text-neutral-100">{t('code_title')}</h3>
      <p className="a-card warn flex items-start gap-2 text-xs" style={{ borderColor: 'var(--warn)' }}>
        <AlertTriangle className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--warn)' }} aria-hidden="true" />
        {t('code_warning')}
      </p>
      {showing && (
        <div className="flex flex-col items-start gap-3">
          <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
            <span className="font-mono text-3xl tracking-[0.2em] text-neutral-50 select-all" aria-label={t('code_label')}>
              {issued.code}
            </span>
            <span className="text-xs text-neutral-400" aria-live="polite">
              {t('code_left', { time: formatCountdown(left) })}
            </span>
          </div>
          <LoginQr link={codeLink(window.location.origin, issued.code)} />
          <p className="text-xs text-neutral-400">{t('code_works_once', { time: formatCountdown(left) })}</p>
          <p className="text-xs text-neutral-400">{t('code_expiry')}</p>
        </div>
      )}
      {issued !== null && !showing && <p className="text-xs text-neutral-400">{t('code_ran_out')}</p>}
      <div className="flex flex-wrap gap-3">
        <button type="button" onClick={start} disabled={busy} className="hq-btn-ghost disabled:opacity-40">
          {showing || issued !== null ? t('code_new') : t('code_title')}
        </button>
        {showing && (
          <button type="button" onClick={() => void navigator.clipboard?.writeText(issued.code)} className="hq-btn-ghost">
            {t('code_copy')}
          </button>
        )}
      </div>
      <ErrorLine message={error} />
    </div>
  )
}

/** "3 Nov": the day a device signed in, short. */
export function shortDay(unixSeconds: number): string {
  return new Date(unixSeconds * MS_PER_SECOND).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

/** The device's name with the shared mark, from what HQ stores; never the raw user agent. */
export function deviceName(d: Pick<SignedInDevice, 'label' | 'shared'>): string {
  return d.shared ? t('device_shared', { device: d.label }) : d.label
}

/** The line under a device: when it signed in and when it was last used. */
export function deviceLine(d: SignedInDevice, nowMs: number): string {
  const signedIn = new Date(d.created_at * MS_PER_SECOND)
  const today = new Date(nowMs)
  if (d.new && signedIn.toDateString() === today.toDateString()) {
    const time = signedIn.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
    return t('device_line_new', { time })
  }
  if (d.last_used_at === null) return t(d.keep ? 'device_line_unused_kept' : 'device_line_unused', { date: shortDay(d.created_at) })
  const vars = { date: shortDay(d.created_at), used: lastUsedText(d.last_used_at, nowMs) }
  return t(d.keep ? 'device_line_kept' : 'device_line', vars)
}

/** One device's row menu: today it holds Keep signed in. */
function RowMenu({ device, busy, onKeep }: { device: SignedInDevice; busy: boolean; onKeep: () => void }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="relative flex-shrink-0">
      <button type="button" className="a-ib" aria-haspopup="menu" aria-expanded={open} aria-label={t('device_menu', { device: device.label })} onClick={() => setOpen(!open)}>
        <MoreHorizontal className="w-4 h-4" aria-hidden="true" />
      </button>
      {open && (
        <div role="menu" className="a-card absolute right-0 z-10 mt-1">
          <button
            type="button"
            role="menuitem"
            className="a-btn quiet"
            disabled={busy}
            onClick={() => {
              setOpen(false)
              onKeep()
            }}
          >
            {t('device_keep')}
          </button>
        </div>
      )}
    </div>
  )
}

/** The other signed-in devices by name; this browser's own sign-out is in This browser. */
function DeviceRows({ list, onChanged }: { list: DeviceList; onChanged: () => void }) {
  const [busyId, setBusyId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const now = Date.now()
  const [asking, setAsking] = useState<SignedInDevice | null>(null)

  const signOut = async (d: SignedInDevice) => {
    setBusyId(d.id)
    setError(null)
    try {
      await signOutDevice(d.id)
      setAsking(null)
      toastAlert(t('sign_out_device_done', { device: d.label }))
      onChanged()
    } catch (e) {
      setError(errorText(e, t('device_out_failed')))
    } finally {
      setBusyId(null)
    }
  }

  const keep = async (d: SignedInDevice) => {
    setError(null)
    try {
      await keepDevice(d.id, true)
      onChanged()
    } catch (e) {
      setError(errorText(e, t('device_keep_failed')))
    }
  }

  const others = list.devices.filter((d) => !d.current)
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-sm font-semibold text-neutral-100">{t('devices_title')}</h3>
      {others.length === 0 && <p className="text-xs text-neutral-400">{t('devices_none')}</p>}
      <ul className="flex flex-col divide-y divide-white/5">
        {others.map((d) => (
          <li key={d.id} className="flex items-center justify-between gap-3 py-2">
            <div className="min-w-0">
              <p className="text-neutral-100 truncate">
                {deviceName(d)}
                {d.new && (
                  <span className="ml-2 inline-flex items-center gap-1 text-xs" style={{ color: 'var(--warn)' }}>
                    <AlertTriangle className="w-3 h-3" aria-hidden="true" />
                    {t('device_new_tag')}
                  </span>
                )}
              </p>
              <p className="text-xs text-neutral-400">{deviceLine(d, now)}</p>
            </div>
            <div className="flex items-center gap-1 flex-shrink-0">
              <button type="button" onClick={() => setAsking(d)} disabled={busyId !== null} className="hq-btn-ghost disabled:opacity-40">
                {d.new ? t('sign_it_out') : t('device_sign_out')}
              </button>
              {!d.keep && <RowMenu device={d} busy={busyId !== null} onKeep={() => void keep(d)} />}
            </div>
          </li>
        ))}
      </ul>
      <p className="text-xs text-neutral-400">{t('device_rule')}</p>
      {asking && (
        <Dialog
          kind="red"
          title={t('sign_out_device_title', { device: asking.label })}
          body={t('sign_out_device_body')}
          keepLabel={t('sign_out_device_keep')}
          actLabel={t('sign_out_device_act')}
          busyLabel={t('confirm_busy_signing_out')}
          busy={busyId !== null}
          error={error}
          onKeep={() => setAsking(null)}
          onAct={() => void signOut(asking)}
        />
      )}
      {list.operator_link_active && <p className="text-xs text-neutral-400">{t('device_operator')}</p>}
      <ErrorLine message={error} />
    </div>
  )
}

/** Sign out every device and the original link, behind a confirm step. */
function SignOutEverywhere() {
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const rotate = async () => {
    setBusy(true)
    setError(null)
    try {
      const token = await signOutEverywhere()
      if (!saveWebToken(token)) throw new Error(t('sign_out_all_blocked'))
      // This browser's saved copy and push go before it loads again with the new token.
      await import('~/lib/wipe').then((m) => m.wipeBrowser()).catch((err) => console.warn('wipe failed', err))
      window.location.reload()
    } catch (e) {
      setError(errorText(e, t('sign_out_all_failed')))
      setBusy(false)
    }
  }

  return (
    <div>
      <button type="button" onClick={() => setConfirming(true)} className="hq-btn-ghost">
        {t('sign_out_all_act')}
      </button>
      <p className="mt-2 text-xs text-neutral-400">{t('device_pair_note')}</p>
      {confirming && (
        <Dialog
          kind="red"
          title={t('sign_out_all_title')}
          body={t('sign_out_all_body')}
          keepLabel={t('sign_out_all_keep')}
          actLabel={t('sign_out_all_act')}
          busyLabel={t('confirm_busy_signing_out')}
          busy={busy}
          error={error}
          onKeep={() => setConfirming(false)}
          onAct={() => void rotate()}
        />
      )}
    </div>
  )
}

/** Sign another device in, see which devices are signed in, and sign them out. */
export function DevicesSection() {
  const [list, setList] = useState<DeviceList | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const signedIn = hasWebToken()

  const load = useCallback(() => {
    listDevices()
      .then((fresh) => {
        setList(fresh)
        setLoadError(null)
      })
      .catch((e) => setLoadError(errorText(e, t('devices_load_failed'))))
  }, [])

  useEffect(() => {
    if (signedIn) load()
  }, [signedIn, load])

  return (
    <section id="devices" aria-labelledby="devices-title" className="scroll-mt-4">
      <h2 id="devices-title" className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        {t('devices_title')}
      </h2>
      <div className="hq-card rounded-2xl px-5 py-4 flex flex-col gap-4 text-sm text-neutral-300">
        {!signedIn ? (
          <p>{t('devices_no_token')}</p>
        ) : (
          <>
            <SignInAnother />
            {list && <DeviceRows list={list} onChanged={load} />}
            <ErrorLine message={loadError} />
            <SignOutEverywhere />
          </>
        )}
      </div>
    </section>
  )
}
