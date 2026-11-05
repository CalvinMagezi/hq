import { t } from '~/strings'
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
  /** What a finished step says in place of its detail. */
  doneDetail?: string
  linkLabel: string
  to: '/settings' | '/inbox'
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
const PLATFORM_NAMES = { telegram: 'Telegram', discord: 'Discord' } as const

/** The first chat app that is connected, by its name, or null. */
export function connectedApp(status: ChatAppsStatus | undefined): string | null {
  if (!status) return null
  const found = CHAT_PLATFORMS.find((p) => status[p].token_set && status[p].owner.paired)
  return found ? PLATFORM_NAMES[found] : null
}

/** A chat app counts once its bot token is saved and the owner has paired with it. */
export function chatAppConnected(status: ChatAppsStatus | undefined): boolean {
  return connectedApp(status) !== null
}

/** "1 of 3 done" for the card's heading. */
export function progressLine(items: GetStartedItem[]): string {
  return t('gs_progress', { done: items.filter((i) => i.done).length, total: items.length })
}

/** The web's real steps, in the order the phone's first run names them. The AI key is not a step. */
export function getStartedItems(s: GetStartedSignals): GetStartedItem[] {
  const items: GetStartedItem[] = []
  // Without a Push API, or with push off in the server config, the owner cannot finish this step from the web.
  if (s.push !== 'unsupported' && s.push !== 'server-off') {
    items.push({
      id: 'notifications',
      title: t('gs_notify_title'),
      detail: t('gs_notify_detail'),
      linkLabel: t('gs_notify_action'),
      to: '/inbox',
      done: s.push === 'on',
    })
  }
  items.push(
    {
      id: 'phone',
      title: t('gs_phone_title'),
      detail: t('gs_phone_detail'),
      linkLabel: t('gs_phone_action'),
      to: '/settings',
      hash: 'devices',
      done: s.standalone || s.prefs.phoneDone,
    },
    {
      id: 'chat',
      title: t('gs_chat_title'),
      detail: t('gs_chat_detail'),
      linkLabel: t('gs_chat_action'),
      to: '/settings',
      hash: 'chat-apps',
      doneDetail: connectedApp(s.chatApps) ? t('gs_chat_detail_done', { app: connectedApp(s.chatApps) ?? '' }) : undefined,
      done: chatAppConnected(s.chatApps),
    },
  )
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
