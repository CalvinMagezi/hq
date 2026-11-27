// Search over Settings: the name of every page and row, its group, and the line under it. A hit jumps to the page.
import type { StringKey } from '~/strings'
import { settingsGroups, type SettingsRowDef, type WhereHqRuns } from './settingsGroups'
import { matchWords } from './search'

export interface SettingsHit {
  row: SettingsRowDef
  group: StringKey
  /** The line under the row's name, which is also where the words matched. */
  hint: StringKey
}

const HINT: Record<string, StringKey> = {
  notifications: 'sethint_notifications',
  time: 'sethint_time',
  version: 'sethint_version',
  data: 'sethint_data',
  signout: 'sethint_signout',
  keys: 'sethint_keys',
  messaging: 'sethint_messaging',
  devices: 'sethint_devices',
  github: 'sethint_github',
  spending: 'sethint_spending',
  safety: 'sethint_safety',
  folders: 'sethint_folders',
  access: 'sethint_access',
  memory: 'sethint_memory',
  backup: 'sethint_backup',
  chat: 'sethint_chat',
  work_tasks: 'sethint_work_tasks',
  work_helpers: 'sethint_work_helpers',
}

/** Every row of Settings with its group and hint, in the order the list shows them. */
export function settingsEntries(where: WhereHqRuns): SettingsHit[] {
  return settingsGroups(where).flatMap((g) => g.rows.map((row) => ({ row, group: g.label, hint: HINT[row.id] })))
}

/** The rows whose name, group or hint holds every word typed; the rows themselves, not a ranking. */
export function searchSettings(where: WhereHqRuns, query: string, name: (key: StringKey) => string): SettingsHit[] {
  if (!query.trim()) return []
  return settingsEntries(where).filter((h) => matchWords(query, name(h.row.label), name(h.group), h.hint ? name(h.hint) : ''))
}
