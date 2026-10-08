// Fixed host: a caller can never point this server at another URL, so it is not an SSRF relay.
const API = 'https://api.hetzner.cloud/v1'
const TIMEOUT_MS = 20_000
const PAGE_SIZE = 50
const MAX_PAGES = 10

export const LABEL_KEY = 'hq-deploy'
export const MANAGED_BY = { 'managed-by': 'agent-hq-deploy' }
export const MANAGED_SELECTOR = 'managed-by=agent-hq-deploy'

/** Selector for the firewall and ssh key the wizard made for one server; both labels must match. */
export const ownedSelector = (name: string) => encodeURIComponent(`${MANAGED_SELECTOR},${LABEL_KEY}=${name}`)

export class HetznerError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code: string,
  ) {
    super(message)
  }
}

interface HetznerErrorBody {
  error?: { code?: string; message?: string }
}

const FRIENDLY: Record<number, string> = {
  401: 'Hetzner rejected that token. Check that it is a read and write token for the project you want.',
  403: 'That token is not allowed to do this. It needs read and write access.',
  404: 'Hetzner could not find that resource.',
  429: 'Hetzner is rate limiting this project. Wait a minute and try again.',
}

export async function hcloud<T>(token: string, path: string, init: { method?: string; body?: unknown } = {}): Promise<T> {
  let res: Response
  try {
    res = await fetch(`${API}${path}`, {
      method: init.method ?? 'GET',
      headers: {
        Authorization: `Bearer ${token}`,
        ...(init.body === undefined ? {} : { 'Content-Type': 'application/json' }),
      },
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
      signal: AbortSignal.timeout(TIMEOUT_MS),
      cache: 'no-store',
    })
  } catch {
    throw new HetznerError('Could not reach Hetzner. Try again in a moment.', 504, 'unreachable')
  }
  const text = await res.text()
  let data: T & HetznerErrorBody
  try {
    data = text ? (JSON.parse(text) as T & HetznerErrorBody) : ({} as T & HetznerErrorBody)
  } catch {
    throw new HetznerError('Hetzner sent an unreadable answer. Try again in a moment.', 502, 'bad_response')
  }
  if (!res.ok) {
    const detail = data.error?.message ?? `Hetzner answered ${res.status}`
    throw new HetznerError(FRIENDLY[res.status] ?? detail, res.status, data.error?.code ?? 'error')
  }
  return data
}

interface Paged {
  meta?: { pagination?: { next_page: number | null } }
}

/** Every page of a list endpoint. `key` is the array's field name in the response. */
export async function listAll<T>(token: string, path: string, key: string): Promise<T[]> {
  const out: T[] = []
  const sep = path.includes('?') ? '&' : '?'
  let page: number | null = 1
  for (let i = 0; page && i < MAX_PAGES; i++) {
    const res: Paged & Record<string, T[]> = await hcloud(token, `${path}${sep}per_page=${PAGE_SIZE}&page=${page}`)
    out.push(...(res[key] ?? []))
    page = res.meta?.pagination?.next_page ?? null
  }
  return out
}
