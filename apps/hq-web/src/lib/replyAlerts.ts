// Opt-in alerts for a reply that finishes while HQ is not on screen. They only
// fire while the page is still running (a background tab, Android). A suspended
// iOS home-screen app needs Web Push, which the server does not send yet.

const PREF_KEY = 'hq.chat.replyAlerts'
const SOUND_URL = '/notification.wav'
const ICON_URL = '/icons/hq-icon-192.png'
const BODY_CHARS = 140

export function alertsSupported() {
  return typeof window !== 'undefined' && 'Notification' in window
}

export function alertsEnabled() {
  if (!alertsSupported() || Notification.permission !== 'granted') return false
  try {
    return localStorage.getItem(PREF_KEY) === '1'
  } catch {
    return false
  }
}

/** Asks for permission (it needs a tap) and remembers the choice. Returns whether alerts are on. */
export async function setAlertsEnabled(on: boolean): Promise<boolean> {
  if (on && alertsSupported() && Notification.permission === 'default') await Notification.requestPermission()
  const enabled = on && alertsSupported() && Notification.permission === 'granted'
  try {
    localStorage.setItem(PREF_KEY, enabled ? '1' : '0')
  } catch {
    // Storage can be blocked; the permission itself still stands.
  }
  return enabled
}

/** Whether a person is looking at the chat right now. */
export function chatOnScreen(overlayOpen: boolean) {
  if (typeof document === 'undefined' || document.visibilityState !== 'visible') return false
  return overlayOpen || window.location.pathname.startsWith('/chat')
}

/** Sound plus a system notification that opens the chat. Android only allows the service-worker kind. */
export async function alertReplyDone(threadId: string, title: string, reply: string) {
  if (!alertsEnabled()) return
  const body = reply.trim().slice(0, BODY_CHARS) || 'Reply finished'
  const options: NotificationOptions = {
    body,
    icon: ICON_URL,
    tag: `hq-chat-${threadId}`,
    data: { url: `/chat?thread=${encodeURIComponent(threadId)}` },
  }
  new Audio(SOUND_URL).play().catch(() => {})
  try {
    const reg = await navigator.serviceWorker?.getRegistration()
    if (reg) await reg.showNotification(title, options)
    else new Notification(title, options)
  } catch (err) {
    console.warn('replyAlerts: could not show a notification', err)
  }
}
