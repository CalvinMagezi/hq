export interface Options {
  callerIp: string
  locations: { name: string; label: string }[]
  serverTypes: { name: string; description: string; cores: number; memoryGb: number; diskGb: number; arch: string; monthlyGross: Record<string, string> }[]
  sshKeys: { id: number; name: string }[]
  existing: ServerInfo[]
}

export interface ServerInfo {
  id: number
  name: string
  status: string
  ip: string | null
}

/** POST to a wizard route. The token travels in the body only, never in a URL or storage. */
export async function call<T>(path: string, body: Record<string, unknown>): Promise<T> {
  const res = await fetch(`/api/hetzner/${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    cache: 'no-store',
  })
  const data = await res.json().catch(() => ({}))
  if (!res.ok) throw new Error(data.error ?? `Request failed (${res.status})`)
  return data as T
}

/** A single-host range from an IPv4 address, or empty. IPv6 is not prefilled: the visitor may reach SSH over IPv4 and lock themselves out. */
export function hostRange(ip: string): string {
  return /^\d{1,3}(\.\d{1,3}){3}$/.test(ip) ? `${ip}/32` : ''
}
