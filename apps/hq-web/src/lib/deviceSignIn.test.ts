import { afterEach, expect, test } from 'bun:test'
import {
  codeLink,
  exchangeCode,
  formatCountdown,
  lastUsedText,
  normalizeCode,
  secondsLeft,
  signOutDevice,
  signOutEverywhere,
} from './deviceSignIn'

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
})

/** Every fetch answers with `status` and `body`; returns what was sent. */
function serverAnswers(status: number, body: unknown) {
  const sent: { url: string; init: RequestInit }[] = []
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    sent.push({ url, init })
    return new Response(JSON.stringify(body), { status })
  }) as unknown as typeof fetch
  return sent
}

test('a code typed in lowercase, with spaces or the hyphen, is the same code', () => {
  expect(normalizeCode(' abcde fgh2j ')).toBe('ABCDEFGH2J')
  expect(normalizeCode('ABCDE-FGH2J')).toBe('ABCDEFGH2J')
})

test('the QR link carries the one-time code in the fragment, never a token or the query', () => {
  const link = codeLink('https://hq.example.ts.net:8443', 'abcde-fgh2j')
  expect(link).toBe('https://hq.example.ts.net:8443/vault#code=ABCDEFGH2J')
  expect(link).not.toContain('token')
})

test('last use reads as a short relative time', () => {
  const now = 1_800_000_000_000
  const secs = now / 1000
  expect(lastUsedText(null, now)).toBe('not used yet')
  expect(lastUsedText(secs - 5, now)).toBe('just now')
  expect(lastUsedText(secs - 5 * 60, now)).toBe('5 min ago')
  expect(lastUsedText(secs - 3 * 3600, now)).toBe('3 h ago')
  expect(lastUsedText(secs - 86_400, now)).toBe('1 day ago')
  expect(lastUsedText(secs - 2 * 86_400, now)).toBe('2 days ago')
  expect(lastUsedText(secs + 60, now)).toBe('just now')
})

test('signing out everywhere returns the new token and refuses an answer without one', async () => {
  const sent = serverAnswers(200, { token: 'hqw_new' })
  expect(await signOutEverywhere()).toBe('hqw_new')
  expect(sent[0].url).toBe('/api/auth/rotate')
  expect(sent[0].init.method).toBe('POST')
  expect(new Headers(sent[0].init.headers).get('X-HQ-Client')).toBe('web')
  serverAnswers(200, {})
  await expect(signOutEverywhere()).rejects.toThrow('without a token')
})

test('signing one device out deletes it by id', async () => {
  const sent: { url: string; init: RequestInit }[] = []
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    sent.push({ url, init })
    return new Response(null, { status: 204 })
  }) as unknown as typeof fetch
  await signOutDevice('a/b')
  expect(sent[0].url).toBe('/api/auth/devices/a%2Fb')
  expect(sent[0].init.method).toBe('DELETE')
})

test('the countdown never goes below zero and shows minutes and seconds', () => {
  expect(secondsLeft(10_500, 10_000)).toBe(1)
  expect(secondsLeft(10_000, 20_000)).toBe(0)
  expect(formatCountdown(600)).toBe('10:00')
  expect(formatCountdown(65)).toBe('1:05')
})

test('a good code comes back as the token, sent without any Authorization header', async () => {
  const sent = serverAnswers(200, { token: 'web-token' })
  expect(await exchangeCode('abcde-fgh2j')).toBe('web-token')
  const headers = new Headers(sent[0].init.headers)
  expect(sent[0].url).toBe('/api/auth/exchange')
  expect(headers.get('X-HQ-Client')).toBe('web')
  expect(headers.has('Authorization')).toBe(false)
  expect(JSON.parse(String(sent[0].init.body))).toEqual({ code: 'ABCDEFGH2J' })
})

test('a short code is refused before it costs an attempt', async () => {
  const sent = serverAnswers(200, { token: 'web-token' })
  await expect(exchangeCode('abc')).rejects.toThrow('10 letters')
  expect(sent).toHaveLength(0)
})

test('a wrong code and too many tries get their own messages', async () => {
  serverAnswers(401, { error: 'that code is not valid' })
  await expect(exchangeCode('ABCDE-FGH2J')).rejects.toThrow('wrong, already used or expired')
  serverAnswers(429, { error: 'too many attempts' })
  await expect(exchangeCode('ABCDE-FGH2J')).rejects.toThrow('Too many tries')
})
