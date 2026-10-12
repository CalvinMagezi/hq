import { useEffect, useState } from 'react'
import { BellOff, BellRing, Loader2, Send } from 'lucide-react'
import { disablePush, enablePush, getPushState, sendTestPush, type PushState } from '~/lib/webPush'

const HINTS: Partial<Record<PushState, string>> = {
  unsupported: 'This browser cannot receive push notifications.',
  'needs-install': 'On iPhone and iPad, add HQ to your Home Screen, open it from there, then turn push on.',
  'server-off': 'Push is off on the server. Set web_push.enabled: true in the HQ config.',
  denied: 'Notifications are blocked for this site. Allow them in the browser settings, then try again.',
}

/** Turns Web Push on or off for this device, on the Notifications page. */
export function PushToggle() {
  const [state, setState] = useState<PushState | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    getPushState()
      .then(setState)
      .catch(() => setState('server-off'))
  }, [])

  const run = async (action: () => Promise<PushState>) => {
    setBusy(true)
    setMessage(null)
    try {
      setState(await action())
    } catch (err) {
      setMessage(err instanceof Error ? err.message : 'Something went wrong.')
    } finally {
      setBusy(false)
    }
  }

  const test = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const error = await sendTestPush()
      setMessage(error ?? 'Test sent. It should arrive in a few seconds.')
    } catch (err) {
      setMessage(err instanceof Error ? err.message : 'The test push failed.')
    } finally {
      setBusy(false)
    }
  }

  if (state === null) return null
  const hint = HINTS[state]

  return (
    <div className="flex flex-col items-start gap-1 mb-5">
      <div className="flex items-center gap-2">
        {state === 'on' ? (
          <>
            <button type="button" onClick={test} disabled={busy} className="hq-btn-ghost" title="Send a test notification to this device">
              <Send className="w-3.5 h-3.5" />
              <span>Test</span>
            </button>
            <button type="button" onClick={() => run(disablePush)} disabled={busy} className="hq-btn-ghost" title="Stop push notifications on this device">
              {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <BellRing className="w-3.5 h-3.5 text-emerald-400" />}
              <span>Push on</span>
            </button>
          </>
        ) : (
          <button
            type="button"
            onClick={() => run(enablePush)}
            disabled={busy || state !== 'off'}
            className="hq-btn-ghost disabled:opacity-50"
            title="Get HQ notifications on this device, even with HQ closed"
          >
            {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <BellOff className="w-3.5 h-3.5" />}
            <span>Turn on push</span>
          </button>
        )}
      </div>
      {(message ?? hint) && <p className="text-[11px] text-neutral-500 max-w-md">{message ?? hint}</p>}
    </div>
  )
}
