import { afterEach, expect, test } from 'bun:test'
import { adoptLinkToken, hasWebToken } from './hqAuth'

const TOKEN = 'a'.repeat(64)
const globals = globalThis as unknown as Record<string, unknown>

function browser(href: string) {
  const store = new Map<string, string>()
  const replaced: string[] = []
  globals.window = { location: { href }, history: { replaceState: (_s: unknown, _t: string, url: string) => replaced.push(url) } }
  globals.localStorage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) }
  return { store, replaced }
}

afterEach(() => {
  delete globals.window
  delete globals.localStorage
})

test('a token in the link is stored and removed from the address bar before any redirect', () => {
  const { store, replaced } = browser(`https://hq.example.ts.net:8443/#token=${TOKEN}`)
  adoptLinkToken()
  expect(store.get('hq-web-token')).toBe(TOKEN)
  expect(replaced[0]).not.toContain(TOKEN)
})

test('a link without a token leaves a stored one alone', () => {
  const { store } = browser('https://hq.example.ts.net:8443/vault')
  store.set('hq-web-token', TOKEN)
  adoptLinkToken()
  expect(hasWebToken()).toBe(true)
  expect(store.get('hq-web-token')).toBe(TOKEN)
})
