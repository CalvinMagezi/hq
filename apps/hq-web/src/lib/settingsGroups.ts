// The four Settings groups, the card slot and the rules around them (web wave 1 spec 06).

import type { StringKey } from '~/strings'

export type SettingsPath =
  | '/settings/browser'
  | '/settings/time'
  | '/settings/version'
  | '/settings/data'
  | '/settings/keys'
  | '/settings/messaging'
  | '/settings/devices'
  | '/settings/github'
  | '/settings/spending'
  | '/settings/safety'
  | '/settings/folders'
  | '/settings/backup'

export interface SettingsRowDef {
  id: string
  label: StringKey
  /** Null for a row that acts instead of opening a page (Sign out this browser). */
  path: SettingsPath | '/helpers' | '/tasks' | null
}

export interface SettingsGroupDef {
  id: 'browser' | 'accounts' | 'behaves' | 'work' | 'data'
  label: StringKey
  rows: SettingsRowDef[]
}

/** The page `/settings` opens on a wide screen, so it is the same section every time (W37). */
export const LANDING_PATH: SettingsPath = '/settings/browser'

const LITE_PATHS: SettingsPath[] = ['/settings/browser', '/settings/time', '/settings/version', '/settings/data', '/settings/devices']

export interface WhereHqRuns {
  lite: boolean
  hub: boolean
}

/** The groups in the phone's order, with the web's own group first. Lite and the HQ window drop what does not apply. */
export function settingsGroups({ lite, hub }: WhereHqRuns): SettingsGroupDef[] {
  const browser: SettingsRowDef[] = [
    { id: 'notifications', label: 'row_notifications', path: '/settings/browser' },
    { id: 'time', label: 'row_time_zone', path: '/settings/time' },
    ...(hub ? [] : [{ id: 'version', label: 'row_install_version', path: '/settings/version' } as SettingsRowDef]),
    { id: 'data', label: 'row_data_used', path: '/settings/data' },
    { id: 'signout', label: hub ? 'row_sign_out_window' : 'row_sign_out_browser', path: null },
  ]
  const groups: SettingsGroupDef[] = [
    { id: 'browser', label: hub ? 'group_window' : 'group_browser', rows: browser },
    {
      id: 'accounts',
      label: 'group_accounts',
      rows: [
        { id: 'keys', label: 'row_ai_keys', path: '/settings/keys' },
        { id: 'messaging', label: 'row_messaging', path: '/settings/messaging' },
        { id: 'devices', label: 'row_other_devices', path: '/settings/devices' },
        { id: 'github', label: 'row_github', path: '/settings/github' },
      ],
    },
    {
      id: 'behaves',
      label: 'group_behaves',
      rows: [
        { id: 'spending', label: 'row_spending', path: '/settings/spending' },
        { id: 'safety', label: 'row_safety', path: '/settings/safety' },
        { id: 'folders', label: 'row_folders', path: '/settings/folders' },
      ],
    },
    {
      // Tasks and Helpers are where HQ and its agents work. They are off the bar, and this is
      // the door for a person who wants to look in.
      id: 'work',
      label: 'group_work',
      rows: [
        { id: 'work_tasks', label: 'row_work_tasks', path: '/tasks' },
        { id: 'work_helpers', label: 'row_work_helpers', path: '/helpers' },
      ],
    },
    { id: 'data', label: 'group_data', rows: [{ id: 'backup', label: 'row_backup', path: '/settings/backup' }] },
  ]
  // Lite keeps its tasks and runs no helpers.
  if (lite) {
    const work = groups.find((g) => g.id === 'work')!
    work.rows = work.rows.filter((r) => r.id === 'work_tasks')
  }
  if (!lite) return groups
  return groups
    .map((g) => ({ ...g, rows: g.rows.filter((r) => r.path === null || r.path === '/tasks' || LITE_PATHS.includes(r.path as SettingsPath)) }))
    .filter((g) => g.rows.length > 0)
}

/** In Lite, a page that HQ Lite lacks opens This browser with a line saying so (WR2, WR28). */
export function liteNotice(path: string, lite: boolean): StringKey | null {
  if (!lite || LITE_PATHS.includes(path as SettingsPath)) return null
  return path === '/settings/spending' ? 'lite_no_spending' : 'lite_only'
}

/** Where each old `#section` link goes now. */
const HASH_PATHS: Record<string, SettingsPath> = {
  clock: '/settings/time',
  devices: '/settings/devices',
  'chat-apps': '/settings/messaging',
  model: '/settings/keys',
  backends: '/settings/keys',
  keys: '/settings/keys',
  credits: '/settings/spending',
  spend: '/settings/spending',
  limits: '/settings/safety',
  safety: '/settings/safety',
  agents: '/settings/safety',
  integrations: '/settings/safety',
  'session-folders': '/settings/folders',
}

export function hashRedirect(hash: string): SettingsPath | null {
  return HASH_PATHS[hash.replace(/^#/, '')] ?? null
}

/** The settings pages are a list and a detail from this width up (1024 px). */
export const WIDE_MIN_PX = 1024

export interface BackupStatus {
  configured: boolean
  last_ok_at: string | null
  failed: boolean
  overdue: boolean
}

/** Amber only when HQ itself reports failed or overdue (B10); an HQ without the route reports nothing. */
export function backupNeedsLook(status: BackupStatus | null): boolean {
  return Boolean(status && status.configured && (status.failed || status.overdue))
}

/** "20 min", "3 h", "3 d": the age of `iso` at `now`, for the sentences that carry one. */
export function ageWords(iso: string, now = Date.now()): string {
  const mins = Math.max(0, Math.floor((now - new Date(iso).getTime()) / 60_000))
  if (mins < 60) return `${mins} min`
  const hours = Math.floor(mins / 60)
  return hours < 48 ? `${hours} h` : `${Math.floor(hours / 24)} d`
}

export type CardId = 'newer' | 'notif_off' | 'backups'

export interface CardInputs {
  newerRunning: boolean
  notificationsOff: boolean
  backupNeedsLook: boolean
}

/** The one card on top: the first that is true, or none. */
export function pickCard(i: CardInputs): CardId | null {
  if (i.newerRunning) return 'newer'
  if (i.notificationsOff) return 'notif_off'
  if (i.backupNeedsLook) return 'backups'
  return null
}

const MB = 1024 * 1024

/** "120 MB", "0.4 MB" or "240 KB". */
export function formatSize(bytes: number): string {
  if (bytes >= 10 * MB) return `${Math.round(bytes / MB)} MB`
  if (bytes >= MB / 10) return `${(bytes / MB).toFixed(1)} MB`
  return `${Math.round(bytes / 1024)} KB`
}

/** The GitHub row's words: who it is connected to, or the step that is left. Null until HQ has answered. */
export interface GithubTrail {
  key: StringKey
  owner?: string
  needsLook: boolean
}

export function githubTrail(link: { standing: string; owner: string | null } | null | undefined): GithubTrail | null {
  if (!link) return null
  if (link.standing === 'connected') return { key: 'github_connected_as', owner: link.owner ?? '', needsLook: false }
  if (link.standing === 'needs_install') return { key: 'trail_github_install', needsLook: true }
  if (link.standing === 'reconnect') return { key: 'trail_github_reconnect', needsLook: true }
  return { key: 'trail_github_off', needsLook: false }
}

/** The Notifications row's words: the two switches only while this browser really receives them. */
export function trailNotifKey(prefs: { needs: boolean; updates: boolean }, pushOn: boolean): StringKey {
  if (!pushOn) return 'trail_notif_none'
  if (prefs.needs) return prefs.updates ? 'trail_notif_both' : 'trail_notif_needs'
  return prefs.updates ? 'trail_notif_updates' : 'trail_notif_none'
}
