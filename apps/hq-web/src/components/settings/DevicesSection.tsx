import { useCallback, useEffect, useRef, useState } from 'react'
import { hasWebToken, saveWebToken } from '~/lib/hqAuth'
import {
  codeLink,
  formatCountdown,
  lastUsedText,
  listDevices,
  requestDeviceCode,
  secondsLeft,
  signOutDevice,
  signOutEverywhere,
  type DeviceList,
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
    <p className="text-xs" style={{ color: 'var(--accent-red)' }}>
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
  if (failed) return <p className="text-xs text-neutral-400">The QR code could not be drawn. Use the code instead.</p>
  return <canvas ref={canvas} width={QR_SIZE_PX} height={QR_SIZE_PX} aria-label="QR code that signs a device in once" className="rounded-lg" />
}

/** A one-time code and its QR code, for signing in a new device. */
function SignInAnother() {
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
      setError(errorText(e, 'Could not get a code.'))
    } finally {
      setBusy(false)
    }
  }

  const showing = issued !== null && left > 0
  return (
    <div className="flex flex-col gap-3">
      <p>On the new device, open HQ and enter the code in the sign-in banner, or scan the QR code with its camera.</p>
      {showing && (
        <div className="flex flex-col items-start gap-3">
          <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
            <span className="font-mono text-3xl tracking-[0.2em] text-neutral-50 select-all" aria-label="Sign-in code">
              {issued.code}
            </span>
            <span className="text-xs text-neutral-400" aria-live="polite">
              works once, for {formatCountdown(left)}
            </span>
          </div>
          <LoginQr link={codeLink(window.location.origin, issued.code)} />
          <p className="text-xs text-neutral-400">The QR code holds the same one-time code: it signs in one device, once.</p>
        </div>
      )}
      {issued !== null && !showing && <p className="text-xs text-neutral-400">That code expired.</p>}
      <div className="flex flex-wrap gap-3">
        <button type="button" onClick={start} disabled={busy} className="hq-btn-ghost disabled:opacity-40">
          {showing ? 'New code' : 'Sign in another device'}
        </button>
        {showing && (
          <button type="button" onClick={() => setIssued(null)} className="hq-btn-ghost">
            Hide
          </button>
        )}
      </div>
      <ErrorLine message={error} />
    </div>
  )
}

/** The signed-in devices, each with a Sign out button. */
function DeviceRows({ list, onChanged }: { list: DeviceList; onChanged: () => void }) {
  const [busyId, setBusyId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const now = Date.now()

  const signOut = async (id: string, current: boolean) => {
    if (current && !window.confirm('Sign this device out? You will need a code, a login link or /login to get back in.')) return
    setBusyId(id)
    setError(null)
    try {
      await signOutDevice(id)
      if (current) window.location.reload()
      else onChanged()
    } catch (e) {
      setError(errorText(e, 'Could not sign that device out.'))
    } finally {
      setBusyId(null)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      {list.devices.length === 0 && <p className="text-xs text-neutral-400">No device has signed in with a code, a QR code or /login yet.</p>}
      <ul className="flex flex-col divide-y divide-white/5">
        {list.devices.map((d) => (
          <li key={d.id} className="flex items-center justify-between gap-3 py-2">
            <div className="min-w-0">
              <p className="text-neutral-100 truncate">
                {d.label}
                {d.current && <span className="ml-2 text-xs text-neutral-400">this device</span>}
              </p>
              <p className="text-xs text-neutral-400">
                Signed in {new Date(d.created_at * MS_PER_SECOND).toLocaleDateString()}, used {lastUsedText(d.last_used_at, now)}
              </p>
            </div>
            <button
              type="button"
              onClick={() => signOut(d.id, d.current)}
              disabled={busyId !== null}
              className="hq-btn-ghost flex-shrink-0 disabled:opacity-40"
            >
              Sign out
            </button>
          </li>
        ))}
      </ul>
      {list.operator_link_active && (
        <p className="text-xs text-neutral-400">
          The original login link from whoever set up your server still works
          {list.this_device_uses_operator_link ? ', and this device uses it' : ''}. Sign out everywhere turns it off.
        </p>
      )}
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
      if (!saveWebToken(token)) throw new Error('Every device was signed out, but this browser blocked saving the new sign-in. Use a code or /login.')
      window.location.reload()
    } catch (e) {
      setError(errorText(e, 'Could not sign out everywhere.'))
      setBusy(false)
    }
  }

  if (!confirming) {
    return (
      <div>
        <button type="button" onClick={() => setConfirming(true)} className="hq-btn-ghost">
          Sign out everywhere
        </button>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-white/10 px-4 py-3">
      <p className="text-neutral-100">Sign out every device?</p>
      <p className="text-xs text-neutral-400">
        Every phone, browser and installed app must sign in again, including this one, which is signed back in for you
        right away. The original login link from whoever set up your server stops working too. Use this when a phone is
        lost or a link was shared by mistake.
      </p>
      <div className="flex flex-wrap gap-3">
        <button type="button" onClick={rotate} disabled={busy} className="hq-btn-ghost disabled:opacity-40" style={{ color: 'var(--accent-red)' }}>
          Yes, sign out everywhere
        </button>
        <button type="button" onClick={() => setConfirming(false)} disabled={busy} className="hq-btn-ghost disabled:opacity-40">
          Cancel
        </button>
      </div>
      <ErrorLine message={error} />
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
      .catch((e) => setLoadError(errorText(e, 'Could not load the signed-in devices.')))
  }, [])

  useEffect(() => {
    if (signedIn) load()
  }, [signedIn, load])

  return (
    <section id="devices" aria-labelledby="devices-title" className="scroll-mt-4">
      <h2 id="devices-title" className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        Devices
      </h2>
      <div className="hq-card rounded-2xl px-5 py-4 flex flex-col gap-4 text-sm text-neutral-300">
        {!signedIn ? (
          <p>This server has no web token, so there is nothing to sign in with. Any browser that can reach it opens HQ.</p>
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
