import { hqJson } from './hqAuth'
import { t } from '~/strings'
import { agentName } from './names'
import { dateByAge } from './time'
import { unixToMs } from './connectorApi'

export interface FolderGrant {
  id: number
  host: string
  path: string
  requested_by: string
  approved_at: number
}

export interface SessionFoldersResponse {
  grants: FolderGrant[]
}

export const SESSION_FOLDERS_QUERY_KEY = ['session-folders']

export const listSessionFolders = () =>
  hqJson<SessionFoldersResponse>('/api/session-folders')

export const revokeSessionFolder = (id: number) =>
  hqJson<{ revoked: number }>(`/api/session-folders/${encodeURIComponent(id)}`, 'DELETE')

/** Formats the approved timestamp and requester for a folder grant row. */
export function formatApprovedText(approvedAt: number, requestedBy: string, nowMs: number = Date.now()): string {
  return t('approved_when', { when: dateByAge(unixToMs(approvedAt), nowMs), name: agentName(requestedBy) })
}

/** Confirmation prompt before revoking access to a folder. */
export function revokeConfirmMessage(path: string, host: string): string {
  return `Stop allowing helpers in ${path} on ${host}?`
}
