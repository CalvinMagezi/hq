// Web Push for this device: subscribe through the service worker and hand the subscription
// to the server, which then pushes inbox items here even while HQ is closed
// (docs/WEB_PUSH.md). The server decides what is pushed; this file only turns it on or off.

import { hqJson } from './hqAuth'

interface ServerPushStatus {
  enabled: boolean
  public_key?: string
  subscriptions?: number
}

export type PushState =
  /** This browser has no Push API (or the page is not a secure context). */
  | 'unsupported'
  /** iOS Safari outside a Home Screen app: push needs the installed app. */
  | 'needs-install'
  /** The server has `web_push.enabled` off. */
  | 'server-off'
  /** The person blocked notifications for this site. */
  | 'denied'
  | 'off'
  | 'on'

export function pushSupported(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.isSecureContext &&
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  )
}

/** iPhone and iPad only allow push for an app added to the Home Screen. */
function iosOutsideHomeScreen(): boolean {
  if (typeof navigator === 'undefined') return false
  const ios = /iPad|iPhone|iPod/.test(navigator.userAgent) ||
    (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
  const standalone = window.matchMedia?.('(display-mode: standalone)').matches ||
    (navigator as Navigator & { standalone?: boolean }).standalone === true
  return ios && !standalone
}

function serverStatus(): Promise<ServerPushStatus> {
  return hqJson<ServerPushStatus>('/api/notifications/push')
}

async function currentSubscription(): Promise<PushSubscription | null> {
  const reg = await navigator.serviceWorker.ready
  return reg.pushManager.getSubscription()
}

function keyBytes(base64url: string): Uint8Array<ArrayBuffer> {
  const padded = base64url.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (base64url.length % 4)) % 4)
  const raw = atob(padded)
  const out = new Uint8Array(new ArrayBuffer(raw.length))
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i)
  return out
}

function sameKey(sub: PushSubscription, publicKey: string): boolean {
  const current = sub.options.applicationServerKey
  if (!current) return false
  const a = new Uint8Array(current)
  const b = keyBytes(publicKey)
  return a.length === b.length && a.every((v, i) => v === b[i])
}

function sendToServer(sub: PushSubscription): Promise<unknown> {
  return hqJson('/api/notifications/push/subscribe', 'POST', sub.toJSON())
}

export async function getPushState(): Promise<PushState> {
  if (iosOutsideHomeScreen()) return 'needs-install'
  if (!pushSupported()) return 'unsupported'
  const server = await serverStatus()
  if (!server.enabled) return 'server-off'
  if (Notification.permission === 'denied') return 'denied'
  const sub = await currentSubscription()
  return sub && Notification.permission === 'granted' ? 'on' : 'off'
}

/** Asks for permission (call it from a tap) and subscribes this device. */
export async function enablePush(): Promise<PushState> {
  if (!pushSupported()) return 'unsupported'
  const server = await serverStatus()
  if (!server.enabled || !server.public_key) return 'server-off'
  const permission = await Notification.requestPermission()
  if (permission !== 'granted') return permission === 'denied' ? 'denied' : 'off'
  const reg = await navigator.serviceWorker.ready
  let sub = await reg.pushManager.getSubscription()
  if (sub && !sameKey(sub, server.public_key)) {
    await sub.unsubscribe()
    sub = null
  }
  sub ??= await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: keyBytes(server.public_key),
  })
  await sendToServer(sub)
  return 'on'
}

export async function disablePush(): Promise<PushState> {
  if (!pushSupported()) return 'unsupported'
  const sub = await currentSubscription()
  if (sub) {
    await hqJson('/api/notifications/push/unsubscribe', 'POST', { endpoint: sub.endpoint }).catch(() => undefined)
    await sub.unsubscribe()
  }
  return 'off'
}

/** Sends a test notification to this device. Resolves with the server's error, if any. */
export async function sendTestPush(): Promise<string | null> {
  const sub = await currentSubscription()
  if (!sub) return 'This device is not subscribed.'
  const res = await hqJson<{ sent: number; results: { sent: boolean; error?: string }[] }>(
    '/api/notifications/push/test',
    'POST',
    { endpoint: sub.endpoint },
  )
  return res.sent > 0 ? null : (res.results[0]?.error ?? 'The push was not delivered.')
}

/**
 * On app start: re-send this device's subscription, so the server has it after a browser
 * rotated it or the server lost it, and drop one made for a VAPID key the server no longer has.
 * Never prompts.
 */
export async function syncPushSubscription(): Promise<void> {
  if (!pushSupported() || Notification.permission !== 'granted') return
  try {
    const sub = await currentSubscription()
    if (!sub) return
    const server = await serverStatus()
    if (!server.enabled || !server.public_key) return
    if (!sameKey(sub, server.public_key)) {
      await sub.unsubscribe()
      const reg = await navigator.serviceWorker.ready
      const fresh = await reg.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: keyBytes(server.public_key),
      })
      await sendToServer(fresh)
      return
    }
    await sendToServer(sub)
  } catch (err) {
    console.warn('webPush: could not sync the subscription', err)
  }
}
