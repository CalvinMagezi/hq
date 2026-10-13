import { useEffect, useState, useSyncExternalStore } from 'react'
import { WifiOff } from 'lucide-react'
import { needsSignIn, onSignInChange } from '~/lib/hqAuth'
import { reachability, UNREACHABLE_GRACE_MS, useDeviceOnline } from '~/lib/reachability'
import { useHQStore } from '~/store/hqStore'

/**
 * Says why nothing is updating. A phone with Tailscale switched off is still online, so without
 * this the only sign was a small "saved" label in the header.
 */
export function ReachabilityBanner() {
  const wsConnected = useHQStore((s) => s.wsConnected)
  const signInNeeded = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  const deviceOnline = useDeviceOnline()
  const [disconnectedSince, setDisconnectedSince] = useState<number | null>(null)
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    setDisconnectedSince(wsConnected ? null : Date.now())
  }, [wsConnected])

  useEffect(() => {
    if (disconnectedSince === null) return
    const id = window.setTimeout(() => setNow(Date.now()), UNREACHABLE_GRACE_MS)
    return () => window.clearTimeout(id)
  }, [disconnectedSince])

  const state = reachability({
    wsConnected,
    signInNeeded,
    deviceOnline,
    disconnectedForMs: disconnectedSince === null ? 0 : now - disconnectedSince,
  })
  if (state === 'ok') return null

  return (
    <div
      role="status"
      className="px-4 py-3 relative z-10"
      style={{ background: 'var(--bg-solid-elevated)', borderBottom: '1px solid var(--border)' }}
    >
      <div className="flex flex-col gap-2 max-w-2xl">
        <div className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>
          <WifiOff className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--accent-amber)' }} />
          {state === 'device_offline' ? 'This device is offline' : "Can't reach your HQ"}
        </div>
        {state === 'device_offline' ? (
          <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
            Screens show what was saved on this device. HQ reconnects once you are back online.
          </p>
        ) : (
          <ul className="text-xs list-disc pl-4 flex flex-col gap-1" style={{ color: 'var(--text-dim)' }}>
            <li>Check that Tailscale is switched on and connected on this device. HQ is only reachable through it.</li>
            <li>If it is, your HQ server may be restarting or updating. Give it a minute.</li>
            <li>
              Still nothing? In the Tailscale admin console, check that your HQ server shows as connected. If it doesn't, ask
              whoever set up your server.
            </li>
          </ul>
        )}
        <div>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="px-3 py-1.5 rounded-lg text-xs font-mono border border-white/10 bg-white/5 text-neutral-100 hover:bg-white/10"
          >
            Try again
          </button>
        </div>
      </div>
    </div>
  )
}
