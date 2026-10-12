import { hqJson } from './hqAuth'

/** What a connected client may reach: the gateway allowlist it is held to. */
export type GrantScope = 'read_only' | 'tasks' | 'full'

export interface ConnectorGrant {
  id: string
  client_name: string
  label: string
  grant_scope: GrantScope
  created_at: number
  expires_at: number
  last_used_at: number | null
}

export type ConnectorStatus =
  | { enabled: false }
  | {
      enabled: true
      url: string
      consent_origin: string
      default_scope: GrantScope
      armed_until: number | null
      grants: ConnectorGrant[]
    }

export interface ConnectRequest {
  client_name: string
  client_id: string
  redirect_host: string
  resource: string
  created_at: number
  expires_at: number
  armed_until: number | null
  default_scope: GrantScope
}

export const SCOPE_CHOICES: { id: GrantScope; title: string; detail: string }[] = [
  {
    id: 'read_only',
    title: 'Read only',
    detail: 'Search and read notes, tasks and session logs. Claude cannot change anything.',
  },
  {
    id: 'tasks',
    title: 'Tasks',
    detail: 'Read and update tasks, and nothing else. No notes, no sessions.',
  },
  {
    id: 'full',
    title: 'Full',
    detail:
      'Every HQ tool, including changing notes and tasks and starting coding agents on your machines. Text Claude reads on a web page or in an email could steer these calls.',
  },
]

export const fetchConnector = () => hqJson<ConnectorStatus>('/api/oauth/connector')
export const armConnector = (minutes?: number) =>
  hqJson<{ armed_until: number }>('/api/oauth/connector/arm', 'POST', minutes ? { minutes } : {})
export const disarmConnector = () => hqJson<{ armed_until: null }>('/api/oauth/connector/disarm', 'POST')
export const connectorOff = () => hqJson<{ revoked: number }>('/api/oauth/connector/off', 'POST')
export const revokeGrant = (id: string) =>
  hqJson<{ revoked: boolean }>(`/api/oauth/grants/${encodeURIComponent(id)}/revoke`, 'POST')

export const fetchConnectRequest = (id: string) =>
  hqJson<ConnectRequest>(`/api/oauth/requests/${encodeURIComponent(id)}`)
export const approveConnectRequest = (id: string, scope: GrantScope, label: string) =>
  hqJson<{ redirect: string }>(`/api/oauth/requests/${encodeURIComponent(id)}/approve`, 'POST', { scope, label })
export const denyConnectRequest = (id: string) =>
  hqJson<{ redirect: string }>(`/api/oauth/requests/${encodeURIComponent(id)}/deny`, 'POST')

const LOOPBACK_HOSTS = new Set(['localhost', '127.0.0.1'])
const CLAUDE_HOSTS = new Set(['claude.ai', 'claude.com'])

/** The server checked the redirect already; the page refuses anything else before navigating, so a bug there cannot turn this page into an open redirect. */
export function safeRedirect(target: string): string | null {
  let url: URL
  try {
    url = new URL(target)
  } catch {
    return null
  }
  const claude = url.protocol === 'https:' && CLAUDE_HOSTS.has(url.hostname)
  const loopback = url.protocol === 'http:' && LOOPBACK_HOSTS.has(url.hostname)
  return claude || loopback ? url.toString() : null
}

const MS_PER_SECOND = 1000

/** Seconds-since-epoch from the server as a local clock time. */
export function clockTime(unixSeconds: number): string {
  return new Date(unixSeconds * MS_PER_SECOND).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export const unixToMs = (unixSeconds: number) => unixSeconds * MS_PER_SECOND
