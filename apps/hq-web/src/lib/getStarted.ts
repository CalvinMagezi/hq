import type { ChatAppsStatus } from './chatAppsApi'
import { HqHttpError } from './hqAuth'
import type { PushState } from './webPush'

/** localStorage key for the home screen's Get started card, kept per device. */
export const GET_STARTED_KEY = 'hq.getStarted'

export interface GetStartedPrefs {
  hidden: boolean
  /** The owner said the phone is done, for devices that never run installed, like a laptop. */
  phoneDone: boolean
}

export const DEFAULT_GET_STARTED_PREFS: GetStartedPrefs = { hidden: false, phoneDone: false }

export function isGetStartedPrefs(v: unknown): v is GetStartedPrefs {
  if (typeof v !== 'object' || v === null) return false
  const p = v as Record<string, unknown>
  return typeof p.hidden === 'boolean' && typeof p.phoneDone === 'boolean'
}

export type GetStartedItemId = 'phone' | 'chat' | 'notifications'

export interface GetStartedItem {
  id: GetStartedItemId
  title: string
  detail: string
  linkLabel: string
  to: '/settings' | '/notifications'
  hash?: string
  done: boolean
}

export interface GetStartedSignals {
  prefs: GetStartedPrefs
  /** This page runs as an installed app. */
  standalone: boolean
  /** Undefined while loading or when the request failed. */
  chatApps: ChatAppsStatus | undefined
  /** Null while unknown or when the check failed. */
  push: PushState | null
}

const CHAT_PLATFORMS = ['telegram', 'discord'] as const

/** A chat app counts once its bot token is saved and the owner has paired with it. */
export function chatAppConnected(status: ChatAppsStatus | undefined): boolean {
  if (!status) return false
  return CHAT_PLATFORMS.some((p) => status[p].token_set && status[p].owner.paired)
}

export function getStartedItems(s: GetStartedSignals): GetStartedItem[] {
  const items: GetStartedItem[] = [
    {
      id: 'phone',
      title: 'Open HQ on your phone',
      detail:
        'Sign the phone in with a code from Devices, then add HQ to its home screen: Share, then Add to Home Screen on iPhone, or the browser menu on Android.',
      linkLabel: 'Devices',
      to: '/settings',
      hash: 'devices',
      done: s.standalone || s.prefs.phoneDone,
    },
    {
      id: 'chat',
      title: 'Connect Telegram or Discord',
      detail: 'Talk to HQ from the chat app you already use. If you are ever signed out, send /login there for a new link.',
      linkLabel: 'Chat apps',
      to: '/settings',
      hash: 'chat-apps',
      done: chatAppConnected(s.chatApps),
    },
  ]
  // Without a Push API, or with push off in the server config, the owner cannot finish this step from the web.
  if (s.push !== 'unsupported' && s.push !== 'server-off') {
    items.push({
      id: 'notifications',
      title: 'Turn on notifications',
      detail: 'Get a notification on this device when HQ needs you, even while the app is closed.',
      linkLabel: 'Notifications',
      to: '/notifications',
      done: s.push === 'on',
    })
  }
  return items
}

export interface GetStartedVisibility {
  /** Every live check has answered, successfully or not. */
  settled: boolean
  /** Signed out or the server is unreachable; other banners explain those. */
  unavailable: boolean
  prefs: GetStartedPrefs
  items: GetStartedItem[]
}

/** Hidden until the checks settle, so a finished checklist never flashes on load. */
export function getStartedVisible(v: GetStartedVisibility): boolean {
  if (!v.settled || v.unavailable || v.prefs.hidden) return false
  return v.items.some((i) => !i.done)
}

// A proxy in front of HQ (Tailscale serve, Caddy) answers with these while HQ is down or restarting.
const GATEWAY_STATUSES = new Set([502, 503, 504])

/** A failed fetch with no HTTP status, or a gateway error, means HQ itself could not be reached. */
export function serverUnreachable(error: unknown): boolean {
  return !(error instanceof HqHttpError) || GATEWAY_STATUSES.has(error.status)
}
