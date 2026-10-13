import { hqJson } from './hqAuth'

export type ChatPlatform = 'telegram' | 'discord'

export interface RelayStatus {
  status: string
  last_seen: string
  detail: string | null
  error: string | null
  /** False when the status was written before this HQ process started. */
  current: boolean
}

export interface ChatAppStatus {
  token_set: boolean
  token_from_env: boolean
  enabled: boolean
  enabled_from_env: boolean
  /** The saved config differs from what the running relay started with. */
  needs_restart: boolean
  owner: { paired: boolean; id: string | null; from_config: boolean }
  pending_code: boolean
  bot: { username: string; application_id: string | null; message_content: boolean | null } | null
  relay: RelayStatus | null
}

export interface ChatAppsStatus {
  /** False when the server has no web token, so it refuses changes over HTTP. */
  writable: boolean
  can_restart: boolean
  restart_hint: string | null
  /** Unix seconds this HQ process started; it changes once a restart has happened. */
  started_at: number
  telegram: ChatAppStatus
  discord: ChatAppStatus
}

export interface VerifyResult {
  ok: boolean
  username?: string
  application_id?: string | null
  message_content?: boolean | null
  error?: string
}

export interface SaveResult extends VerifyResult {
  can_restart: boolean
}

export interface PairResult {
  code: string
  command: string
  expires_at: number
  deep_link: string | null
}

export const CHAT_APPS_QUERY_KEY = ['chat-apps']

export const fetchChatApps = () => hqJson<ChatAppsStatus>('/api/chat-apps')
/** Without a token, re-checks the one already saved. */
export const verifyChatToken = (platform: ChatPlatform, token?: string) =>
  hqJson<VerifyResult>(`/api/chat-apps/${platform}/verify`, 'POST', token === undefined ? {} : { token })
export const saveChatToken = (platform: ChatPlatform, token: string) =>
  hqJson<SaveResult>(`/api/chat-apps/${platform}`, 'POST', { token })
export const disconnectChatApp = (platform: ChatPlatform) =>
  hqJson<{ ok: boolean; can_restart: boolean }>(`/api/chat-apps/${platform}`, 'DELETE')
export const pairChatApp = (platform: ChatPlatform, replace = false) =>
  hqJson<PairResult>(`/api/chat-apps/${platform}/pair`, 'POST', { replace })
export const restartHq = () => hqJson<{ restarting: boolean }>('/api/system/restart', 'POST')

/**
 * Discord permissions the relay uses, as the invite's integer: VIEW_CHANNEL (1<<10),
 * SEND_MESSAGES (1<<11), EMBED_LINKS (1<<14), READ_MESSAGE_HISTORY (1<<16), ADD_REACTIONS (1<<6),
 * CREATE_PUBLIC_THREADS (1<<35) and SEND_MESSAGES_IN_THREADS (1<<38). Written out because JS
 * bitwise operators stop at 32 bits.
 */
export const DISCORD_BOT_PERMISSIONS = '309237730368'

export function discordInviteUrl(applicationId: string): string {
  const params = new URLSearchParams({
    client_id: applicationId,
    scope: 'bot applications.commands',
    permissions: DISCORD_BOT_PERMISSIONS,
  })
  return `https://discord.com/oauth2/authorize?${params.toString().replace(/\+/g, '%20')}`
}

const MS_PER_SECOND = 1000
const SECONDS_PER_MINUTE = 60

export function secondsLeft(expiresAt: number, nowMs: number): number {
  return Math.max(0, Math.floor(expiresAt - nowMs / MS_PER_SECOND))
}

/** `m:ss` for a pairing code's remaining life. */
export function formatCountdown(seconds: number): string {
  const m = Math.floor(seconds / SECONDS_PER_MINUTE)
  const s = String(seconds % SECONDS_PER_MINUTE).padStart(2, '0')
  return `${m}:${s}`
}

const RESTART_POLL_MS = 2000
const RESTART_TIMEOUT_MS = 120_000

/**
 * Waits until a new HQ process answers. The first polls may still reach the old one, so this
 * waits for `started_at` to change rather than for any answer.
 */
export async function waitForRestart(previousStartedAt: number, fetchStatus = fetchChatApps): Promise<boolean> {
  const deadline = Date.now() + RESTART_TIMEOUT_MS
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, RESTART_POLL_MS))
    try {
      if ((await fetchStatus()).started_at !== previousStartedAt) return true
    } catch {
      // Down while it restarts; keep waiting.
    }
  }
  return false
}
