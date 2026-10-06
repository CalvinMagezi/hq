// Web token for hq's /ws and /api when the server sets web_auth_token.
// A browser picks it up once from a `#token=` link (the fragment never reaches
// the server or its logs) and keeps it. With no token configured this is a no-op.

import { markSynced } from './offlineCache'

const STORAGE_KEY = 'hq-web-token'
const LINK_PARAM = 'token'

/** Take a token from the link once, then drop it from the address bar. */
function tokenFromLink(url: URL): string | null {
  const hash = new URLSearchParams(url.hash.replace(/^#/, ''))
  const fromHash = hash.get(LINK_PARAM)
  // Older links put it in the query; still read, but it has already been logged upstream.
  const fromQuery = url.searchParams.get(LINK_PARAM)
  const token = fromHash || fromQuery
  if (!token) return null
  url.searchParams.delete(LINK_PARAM)
  hash.delete(LINK_PARAM)
  url.hash = hash.toString()
  window.history.replaceState(null, '', url.toString())
  return token
}

function browserToken(): string | null {
  try {
    const fromLink = tokenFromLink(new URL(window.location.href))
    if (fromLink) {
      localStorage.setItem(STORAGE_KEY, fromLink)
      return fromLink
    }
    return localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

function hqToken(): string | null {
  if (typeof window === 'undefined') return null
  return browserToken()
}

/** Whether requests need the token, which means plain URLs will not load. */
export function hasWebToken(): boolean {
  return hqToken() !== null
}

/** Authorization header for clients that take headers but not a fetch, like pdf.js. */
export function authHeaders(): Record<string, string> {
  const token = hqToken()
  return token ? { Authorization: `Bearer ${token}` } : {}
}

/** Header the server requires on session-changing requests; a cross-site form or fetch cannot add it without a CORS preflight. */
const CLIENT_HEADER = 'X-HQ-Client'
const CLIENT_HEADER_VALUE = 'web'

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS'])

/** fetch() with the web token attached as a Bearer header when one is set. */
export async function hqFetch(input: string, init: RequestInit = {}): Promise<Response> {
  const token = hqToken()
  const headers = new Headers(init.headers)
  if (token) headers.set('Authorization', `Bearer ${token}`)
  if (!SAFE_METHODS.has((init.method ?? 'GET').toUpperCase())) headers.set(CLIENT_HEADER, CLIENT_HEADER_VALUE)
  const res = await fetch(input, { ...init, headers })
  // A copy the service worker served from its cache is not a sync, whatever its status.
  if (res.ok && !res.headers.has('x-hq-cache')) markSynced()
  return res
}

const HTTP_NO_CONTENT = 204

/** A non-2xx API response, carrying the server's `error` message and the status. */
export class HqHttpError extends Error {
  constructor(message: string, readonly status: number) {
    super(message)
  }
}

/** Parse an API response as JSON, throwing HqHttpError on a non-2xx status. */
export async function readJson<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new HqHttpError(data.error || `${res.status} ${res.statusText}`, res.status)
  }
  if (res.status === HTTP_NO_CONTENT) return undefined as T
  return res.json()
}

/** JSON request helper: sends `body` as JSON when given. */
export async function hqJson<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  const init: RequestInit = { method }
  if (body !== undefined) {
    init.headers = { 'Content-Type': 'application/json' }
    init.body = JSON.stringify(body)
  }
  return readJson<T>(await hqFetch(url, init))
}

/**
 * URL for the chat socket. WebSockets cannot carry headers, so with a token set
 * the socket gets a single-use ticket that expires within seconds.
 */
export async function socketUrl(base: string): Promise<string> {
  if (!hasWebToken()) return base
  const { ticket } = await hqJson<{ ticket: string }>('/api/ws-ticket', 'POST')
  const sep = base.includes('?') ? '&' : '?'
  return `${base}${sep}ticket=${encodeURIComponent(ticket)}`
}
