import { afterEach, expect, test } from 'bun:test'
import { adoptLinkToken, hasWebToken, hqFetch, needsSignIn, onSignInChange, saveWebToken, takeLinkCode } from './hqAuth'

const TOKEN = 'a'.repeat(64)
const globals = globalThis as unknown as Record<string, unknown>

function browser(href: string) {
  const store = new Map<string, string>()
  const replaced: string[] = []
  globals.window = { location: { href }, history: { replaceState: (_s: unknown, _t: string, url: string) => replaced.push(url) } }
  globals.localStorage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) }
  return { store, replaced }
}

const realFetch = globalThis.fetch

/** Every fetch answers with this status, plus the service worker's cache marker when `cached`. */
function serverAnswers(status: number, cached = false) {
  const sent: Headers[] = []
  globalThis.fetch = (async (_input: unknown, init?: RequestInit) => {
    sent.push(new Headers(init?.headers))
    return new Response(null, { status, headers: cached ? { 'x-hq-cache': '1' } : {} })
  }) as typeof fetch
  return sent
}

afterEach(async () => {
  // Leave the shared sign-in state clear for the next test.
  serverAnswers(200)
  await hqFetch('/api/spaces')
  globalThis.fetch = realFetch
  delete globals.window
  delete globals.localStorage
})

test('a token in the link is stored and removed from the address bar before any redirect', () => {
  const { store, replaced } = browser(`https://hq.example.ts.net:8443/#token=${TOKEN}`)
  adoptLinkToken()
  expect(store.get('hq-web-token')).toBe(TOKEN)
  expect(replaced[0]).not.toContain(TOKEN)
})

test('a code in the link is held once for the banner and removed from the address bar', () => {
  const { store, replaced } = browser('https://hq.example.ts.net:8443/vault#code=ABCDEFGH2J')
  adoptLinkToken()
  expect(replaced.at(-1)).not.toContain('ABCDEFGH2J')
  expect(store.has('hq-web-token')).toBe(false)
  expect(takeLinkCode()).toBe('ABCDEFGH2J')
  expect(takeLinkCode()).toBeNull()
})

test('a link without a token leaves a stored one alone', () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  store.set('hq-web-token', TOKEN)
  adoptLinkToken()
  expect(hasWebToken()).toBe(true)
  expect(store.get('hq-web-token')).toBe(TOKEN)
})

test('a 401 from the server asks for sign-in, and the next good answer clears it', async () => {
  browser('https://hq.example.ts.net:8443/vault')
  let changes = 0
  const off = onSignInChange(() => changes++)
  serverAnswers(401)
  await hqFetch('/api/spaces')
  expect(needsSignIn()).toBe(true)
  serverAnswers(200)
  await hqFetch('/api/spaces')
  expect(needsSignIn()).toBe(false)
  expect(changes).toBe(2)
  off()
})

test('a copy the service worker served from its cache says nothing about sign-in', async () => {
  browser('https://hq.example.ts.net:8443/vault')
  serverAnswers(401, true)
  await hqFetch('/api/notifications')
  expect(needsSignIn()).toBe(false)
  serverAnswers(401)
  await hqFetch('/api/notifications')
  serverAnswers(200, true)
  await hqFetch('/api/notifications')
  expect(needsSignIn()).toBe(true)
})

test('a refused ticket request for the chat socket asks for sign-in', async () => {
  const { store } = browser('https://hq.example.ts.net:8443/chat')
  store.set('hq-web-token', TOKEN)
  serverAnswers(401)
  await hqFetch('/api/ws-ticket', { method: 'POST' })
  expect(needsSignIn()).toBe(true)
})

test('a pasted token replaces a rotated one and is sent from then on', async () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  store.set('hq-web-token', 'old-token')
  expect(saveWebToken(`  ${TOKEN}\n`)).toBe(true)
  expect(store.get('hq-web-token')).toBe(TOKEN)
  const sent = serverAnswers(200)
  await hqFetch('/api/spaces')
  expect(sent[0].get('Authorization')).toBe(`Bearer ${TOKEN}`)
})

test('a pasted login link stores only its token', () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  expect(saveWebToken(`https://hq.example.ts.net:8443/vault#token=${TOKEN}`)).toBe(true)
  expect(store.get('hq-web-token')).toBe(TOKEN)
})

test('a link with no token after the marker stores nothing', () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  expect(saveWebToken('https://hq.example.ts.net:8443/vault#token=')).toBe(false)
  expect(store.has('hq-web-token')).toBe(false)
})

test('a blank paste stores nothing', () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  expect(saveWebToken('   ')).toBe(false)
  expect(store.has('hq-web-token')).toBe(false)
})
