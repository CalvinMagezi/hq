import { useSyncExternalStore } from 'react'

// Why HQ can't be reached, for the banner that says so. A sign-in problem has its own prompt.

/** Long enough for a normal connect or a quick restart to finish without a flash of the banner. */
export const UNREACHABLE_GRACE_MS = 8000

export type Reachability = 'ok' | 'device_offline' | 'server_unreachable'

export interface ReachabilityInput {
  wsConnected: boolean
  signInNeeded: boolean
  deviceOnline: boolean
  disconnectedForMs: number
}

export function reachability(s: ReachabilityInput): Reachability {
  if (s.wsConnected || s.signInNeeded) return 'ok'
  if (s.disconnectedForMs < UNREACHABLE_GRACE_MS) return 'ok'
  return s.deviceOnline ? 'server_unreachable' : 'device_offline'
}

function onOnlineChange(listener: () => void): () => void {
  window.addEventListener('online', listener)
  window.addEventListener('offline', listener)
  return () => {
    window.removeEventListener('online', listener)
    window.removeEventListener('offline', listener)
  }
}

/** Whether the browser reports a network, read live; the build's prerender counts as online. */
export function useDeviceOnline(): boolean {
  return useSyncExternalStore(onOnlineChange, () => navigator.onLine, () => true)
}
