import { afterEach, beforeEach, expect, test } from 'bun:test'
import { POST as create } from './create/route'
import { POST as remove } from './delete/route'
import { POST as sshRule } from './ssh-rule/route'

const TOKEN = 't'.repeat(64)
const REF = 'a'.repeat(40)
const realFetch = globalThis.fetch
let calls: { method: string; path: string; body: unknown; auth: string | null }[]
let handler: (method: string, path: string) => { status?: number; json: unknown }

beforeEach(() => {
  process.env.HQ_BOOTSTRAP_REF = REF
  process.env.HQ_BOOTSTRAP_SHA256 = 'b'.repeat(64)
  calls = []
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    const path = new URL(url).pathname.replace('/v1', '') + new URL(url).search
    const method = init.method ?? 'GET'
    calls.push({ method, path, body: init.body ? JSON.parse(String(init.body)) : undefined, auth: new Headers(init.headers).get('authorization') })
    const out = handler(method, path)
    return new Response(JSON.stringify(out.json), { status: out.status ?? 200 })
  }) as typeof fetch
})
afterEach(() => {
  globalThis.fetch = realFetch
})

const post = (body: unknown) => new Request('http://x/api', { method: 'POST', body: JSON.stringify(body) })
const GOOD = { token: TOKEN, name: 'hq', location: 'nbg1', serverType: 'cx22', adminCidr: '203.0.113.7/32', sshKeyId: 7 }

test('create makes the firewall first, sends user-data without the token, and returns no secret', async () => {
  handler = (m, p) =>
    p === '/firewalls' ? { json: { firewall: { id: 11 } } } : { json: { server: { id: 5, name: 'hq', public_net: { ipv4: { ip: '198.51.100.1' } } } } }
  const res = await create(post(GOOD))
  expect(res.status).toBe(200)
  expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual(['POST /firewalls', 'POST /servers'])
  const server = calls[1].body as { user_data: string; firewalls: unknown; ssh_keys: number[] }
  expect(server.user_data).toContain(REF)
  expect(server.user_data).not.toContain(TOKEN)
  expect(server.ssh_keys).toEqual([7])
  expect(JSON.stringify(await res.json())).not.toContain(TOKEN)
  expect(calls.every((c) => c.auth === `Bearer ${TOKEN}`)).toBe(true)
})

test('a failed server create removes the firewall and the key it made', async () => {
  handler = (m, p) => {
    if (p === '/ssh_keys') return { json: { ssh_key: { id: 9 } } }
    if (p === '/firewalls') return { json: { firewall: { id: 11 } } }
    if (p === '/servers') return { status: 422, json: { error: { code: 'invalid_input', message: 'bad size' } } }
    return { json: {} }
  }
  const res = await create(post({ ...GOOD, sshKeyId: undefined, newSshKey: { name: 'hq-hq', publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc me' } }))
  expect(res.status).toBe(502)
  const cleanup = calls.filter((c) => c.method === 'DELETE').map((c) => c.path)
  expect(cleanup).toEqual(['/firewalls/11', '/ssh_keys/9'])
})

test('input that would open SSH to everyone or carry a private key never reaches Hetzner', async () => {
  handler = () => ({ json: {} })
  expect((await create(post({ ...GOOD, adminCidr: '0.0.0.0/0' }))).status).toBe(400)
  expect((await create(post({ ...GOOD, sshKeyId: undefined, newSshKey: { name: 'k', publicKey: '-----BEGIN OPENSSH PRIVATE KEY-----' } }))).status).toBe(400)
  expect((await create(post({ ...GOOD, token: 'short' }))).status).toBe(400)
  expect(calls).toHaveLength(0)
})

test('delete refuses a server the wizard did not create and a wrong confirmation', async () => {
  handler = () => ({ json: { server: { id: 5, name: 'prod-db', labels: {} } } })
  expect((await remove(post({ token: TOKEN, serverId: 5, confirmName: 'prod-db' }))).status).toBe(400)
  handler = () => ({ json: { server: { id: 5, name: 'hq', labels: { 'managed-by': 'agent-hq-deploy', 'hq-deploy': 'hq' } } } })
  expect((await remove(post({ token: TOKEN, serverId: 5, confirmName: 'nope' }))).status).toBe(400)
  expect(calls.some((c) => c.method === 'DELETE')).toBe(false)
})

const OWNED = { 'managed-by': 'agent-hq-deploy', 'hq-deploy': 'hq' }

test('delete does not claim success or touch the firewall when the server action fails', async () => {
  handler = (m, p) => {
    if (p === '/servers/5' && m === 'GET') return { json: { server: { id: 5, name: 'hq', labels: OWNED } } }
    if (p === '/servers/5') return { json: { action: { id: 77 } } }
    if (p === '/actions/77') return { json: { action: { status: 'error' } } }
    return { json: {} }
  }
  const res = await remove(post({ token: TOKEN, serverId: 5, confirmName: 'hq' }))
  expect(await res.json()).toEqual({ deleted: false, pending: true })
  expect(calls.filter((c) => c.method === 'DELETE' && c.path !== '/servers/5')).toHaveLength(0)
})

test('firewall and key lookups need both the managed label and the server label', async () => {
  handler = (m, p) => {
    if (p === '/servers/5') return { json: { server: { id: 5, name: 'hq', labels: OWNED } } }
    if (p.startsWith('/firewalls?')) return { json: { firewalls: [{ id: 11 }] } }
    return { json: {} }
  }
  expect((await sshRule(post({ token: TOKEN, serverId: 5 }))).status).toBe(200)
  const lookup = calls.find((c) => c.path.startsWith('/firewalls?'))!
  expect(decodeURIComponent(lookup.path)).toContain('managed-by=agent-hq-deploy,hq-deploy=hq')
  expect(calls.find((c) => c.path.endsWith('/set_rules'))?.body).toEqual({ rules: [] })
})

test('reopening SSH refuses a range wider than the limit', async () => {
  handler = () => ({ json: { server: { id: 5, name: 'hq', labels: OWNED } } })
  expect((await sshRule(post({ token: TOKEN, serverId: 5, openFrom: '128.0.0.0/1' }))).status).toBe(400)
})

test('a timeout creating the server leaves the firewall alone', async () => {
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    const path = new URL(url).pathname.replace('/v1', '')
    calls.push({ method: init.method ?? 'GET', path, body: undefined, auth: null })
    if (path === '/firewalls') return new Response(JSON.stringify({ firewall: { id: 11 } }))
    throw new TypeError('network down')
  }) as unknown as typeof fetch
  const res = await create(post(GOOD))
  expect(res.status).toBe(502)
  expect((await res.json()).error).toContain("check for an existing server")
  expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(0)
})

test('delete that outlives the wait reports pending and removes nothing else', async () => {
  handler = (m, p) => {
    if (p === '/servers/5' && m === 'GET') return { json: { server: { id: 5, name: 'hq', labels: OWNED } } }
    if (p === '/servers/5') return { json: { action: { id: 77 } } }
    if (p === '/actions/77') return { json: { action: { status: 'running' } } }
    return { json: {} }
  }
  const real = globalThis.setTimeout
  globalThis.setTimeout = ((fn: () => void) => real(fn, 0)) as unknown as typeof setTimeout
  try {
    const res = await remove(post({ token: TOKEN, serverId: 5, confirmName: 'hq' }))
    expect(await res.json()).toEqual({ deleted: false, pending: true })
  } finally {
    globalThis.setTimeout = real
  }
  expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(1)
})

test('calling delete again once the server is gone finishes the cleanup', async () => {
  handler = (m, p) => {
    if (p === '/servers/5') return { status: 404, json: { error: { code: 'not_found', message: 'gone' } } }
    if (p.startsWith('/servers?')) return { json: { servers: [] } }
    if (p.startsWith('/firewalls?')) return { json: { firewalls: [{ id: 11 }] } }
    if (p.startsWith('/ssh_keys?')) return { json: { ssh_keys: [{ id: 9 }] } }
    return { json: {} }
  }
  const res = await remove(post({ token: TOKEN, serverId: 5, confirmName: 'hq' }))
  expect(await res.json()).toEqual({ deleted: true, firewallsRemoved: true })
  expect(calls.filter((c) => c.method === 'DELETE').map((c) => c.path)).toEqual(['/firewalls/11', '/ssh_keys/9'])
  expect(decodeURIComponent(calls.find((c) => c.path.startsWith('/firewalls?'))!.path)).toContain('managed-by=agent-hq-deploy,hq-deploy=hq')
})

test('cleanup is skipped while a server with that name still exists', async () => {
  handler = (m, p) => {
    if (p === '/servers/5') return { status: 404, json: { error: { code: 'not_found', message: 'gone' } } }
    if (p.startsWith('/servers?')) return { json: { servers: [{ id: 6 }] } }
    return { json: {} }
  }
  const res = await remove(post({ token: TOKEN, serverId: 5, confirmName: 'hq' }))
  expect(await res.json()).toEqual({ deleted: true, firewallsRemoved: false })
  expect(calls.some((c) => c.method === 'DELETE')).toBe(false)
})
