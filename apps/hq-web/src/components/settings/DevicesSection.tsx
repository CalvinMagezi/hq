import { useEffect, useRef, useState } from 'react'
import { hasWebToken, webToken } from '~/lib/hqAuth'
import { formatCountdown, loginLink, requestDeviceCode, secondsLeft } from '~/lib/deviceSignIn'

const TICK_MS = 1000
const MS_PER_SECOND = 1000
const QR_SIZE_PX = 200
const QR_MARGIN_MODULES = 2

interface Issued {
  code: string
  deadlineMs: number
}

/** Draws the login link as a QR code; the library loads only when someone asks for one. */
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
  return <canvas ref={canvas} width={QR_SIZE_PX} height={QR_SIZE_PX} aria-label="QR code of your HQ login link" className="rounded-lg" />
}

/** Sign another device in with a one-time code or a QR code of the login link. */
export function DevicesSection() {
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
      setError(e instanceof Error ? e.message : 'Could not get a code.')
    } finally {
      setBusy(false)
    }
  }

  const token = webToken()
  const showing = issued !== null && left > 0
  return (
    <section id="devices" aria-labelledby="devices-title" className="scroll-mt-4">
      <h2 id="devices-title" className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        Devices
      </h2>
      <div className="hq-card rounded-2xl px-5 py-4 flex flex-col gap-3 text-sm text-neutral-300">
        {!hasWebToken() ? (
          <p>This server has no web token, so there is nothing to sign in with. Any browser that can reach it opens HQ.</p>
        ) : (
          <>
            <p>
              On the new device, open HQ and enter the code in the sign-in banner, or scan the QR code with its camera.
            </p>
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
                {token && <LoginQr link={loginLink(window.location.origin, token)} />}
                <p className="text-xs text-neutral-400">The QR code is a password: anyone who scans it is signed in to your HQ.</p>
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
          </>
        )}
        {error && <p className="text-xs" style={{ color: 'var(--accent-red)' }}>{error}</p>}
      </div>
    </section>
  )
}
