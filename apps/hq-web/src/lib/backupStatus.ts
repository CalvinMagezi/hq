import { hqJson } from './hqAuth'
import type { BackupStatus } from './settingsGroups'

export const BACKUP_STATUS_KEY = ['backup-status']

/** HQ's backup status, or null from an HQ without the route. */
export function fetchBackupStatus(): Promise<BackupStatus | null> {
  return hqJson<BackupStatus>('/api/backup/status').catch(() => null)
}
