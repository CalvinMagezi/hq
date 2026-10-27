import { hqJson } from './hqAuth'
import { clockTime } from './connectorApi'

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
export function formatApprovedText(approvedAt: number, requestedBy: string): string {
  return `approved ${clockTime(approvedAt)} for ${requestedBy}`
}

/** Confirmation prompt before revoking access to a folder. */
export function revokeConfirmMessage(path: string, host: string): string {
  return `Stop allowing sessions in ${path} on ${host}?`
}
