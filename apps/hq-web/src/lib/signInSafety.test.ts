import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SignInSheet } from '~/components/SignInPrompt'
import { SignInAnother, deviceLine, deviceName } from '~/components/settings/DevicesSection'
import { SignInActions, SignInBody, signInTime } from '~/components/notifications/SignInItem'
import { t } from '~/strings'
import { exchangeCode, exchangeErrorMessage, keepDevice, signInWithCode, signOutCurrentDevice, type SignedInDevice } from './deviceSignIn'
import { needsSignIn, wasSignedOut } from './hqAuth'

const g = globalThis as unknown as Record<string, unknown>
const realFetch = globalThis.fetch

function storage() {
  const m = new Map<string, string>()
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k), keys: () => [...m.keys()] } as unknown as Storage & { keys: () => string[] }
}

let sent: { url: string; init: RequestInit }[] = []
function serverAnswers(status: number, body: unknown) {
  sent = []
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    sent.push({ url, init })
    return new Response(JSON.stringify(body), { status })
  }) as unknown as typeof fetch
}

let opened = 0
beforeEach(() => {
  g.localStorage = storage()
  g.sessionStorage = storage()
  opened = 0
  g.indexedDB = { open: () => void opened++ }
})
afterEach(() => {
  globalThis.fetch = realFetch
  delete g.localStorage
  delete g.sessionStorage
  delete g.indexedDB
})

const device = (over: Partial<SignedInDevice> = {}): SignedInDevice => ({
  id: 'd1',
  label: 'Firefox on Windows',
  created_at: Date.UTC(2026, 10, 3, 14, 20) / 1000,
  last_used_at: null,
  expires_at: null,
  browser: 'Firefox',
  system: 'Windows',
  shared: false,
  keep: false,
  idle_expires_after: 90 * 86_400,
  new: false,
  current: false,
  ...over,
})

describe('the exchange (checks 4 and 5)', () => {
  test('sends the code and the shared choice, nothing else', async () => {
    serverAnswers(200, { token: 'hqw_new' })
    await exchangeCode('abcde-fgh2j', true)
    expect(JSON.parse(String(sent[0].init.body))).toEqual({ code: 'ABCDEFGH2J', shared: true })
  })

  test('on a shared computer the token is in sessionStorage, not localStorage, and no IndexedDB opens', async () => {
    serverAnswers(200, { token: 'hqw_new' })
    await signInWithCode('abcdefgh2j', true)
    expect((g.sessionStorage as ReturnType<typeof storage>).getItem('hq-web-token')).toBe('hqw_new')
    expect((g.localStorage as ReturnType<typeof storage>).getItem('hq-web-token')).toBeNull()
    expect(opened).toBe(0)
  })

  test('a code that is not shared keeps the token in localStorage', async () => {
    serverAnswers(200, { token: 'hqw_new' })
    await signInWithCode('abcdefgh2j', false)
    expect((g.localStorage as ReturnType<typeof storage>).getItem('hq-web-token')).toBe('hqw_new')
  })

  test('a 401 from the exchange says the code did not work and wipes nothing', async () => {
    const local = g.localStorage as ReturnType<typeof storage>
    local.setItem('hq-web-token', 'hqw_old')
    local.setItem('hq-drafts', 'kept')
    serverAnswers(401, {})
    await expect(exchangeCode('abcdefgh2j')).rejects.toThrow(t('signin_wrong_code'))
    expect(local.getItem('hq-web-token')).toBe('hqw_old')
    expect(local.getItem('hq-drafts')).toBe('kept')
    expect(needsSignIn()).toBe(false)
    expect(wasSignedOut()).toBe(false)
  })

  test('refusals use the catalogue words, with no status number', () => {
    expect(exchangeErrorMessage(401)).toBe('That code did not work. Codes last ten minutes and work once. Get a new one.')
    expect(exchangeErrorMessage(429)).toBe('Too many tries. Wait a few minutes, then get a new code.')
    expect(exchangeErrorMessage(500)).not.toMatch(/\d{3}/)
  })

  test('Keep signed in and Sign out this browser call the new routes', async () => {
    serverAnswers(204, {})
    await keepDevice('d 1', true)
    expect(sent[0].url).toBe('/api/auth/devices/d%201/keep')
    expect(JSON.parse(String(sent[0].init.body))).toEqual({ keep: true })
    await signOutCurrentDevice()
    expect(sent[1].url).toBe('/api/auth/devices/current')
    expect(sent[1].init.method).toBe('DELETE')
  })
})

describe('the sheet and the code card', () => {
  test('the sheet has the shared switch, the warning and the sign-in words', () => {
    const html = renderToStaticMarkup(createElement(SignInSheet))
    expect(html).toContain('role="switch"')
    for (const key of ['signin_title', 'shared_computer_label', 'signin_shared_line', 'signin_phishing', 'signin_where_code', 'signin_else'] as const) {
      expect(html).toContain(t(key).replace(/'/g, '&#x27;'))
    }
  })

  test('the code card carries the warning and the 90 day line (check 6)', () => {
    const html = renderToStaticMarkup(createElement(SignInAnother))
    expect(html).toContain('Type this only into HQ on your own device.')
    expect(html).toContain(t('code_warning'))
  })
})

describe('device rows', () => {
  const now = Date.UTC(2026, 10, 5, 12, 0)
  test('a device is named by browser and system, shared ones say so', () => {
    expect(deviceName(device())).toBe('Firefox on Windows')
    expect(deviceName(device({ shared: true }))).toBe('Firefox on Windows, shared')
  })
  test('a device reads as signed in, used, and kept signed in', () => {
    expect(deviceLine(device({ last_used_at: now / 1000 }), now)).toBe('Signed in 3 Nov · used just now')
    expect(deviceLine(device({ keep: true, last_used_at: now / 1000 }), now)).toBe('Signed in 3 Nov · used just now · kept signed in')
  })
})

describe('the new sign-in card (check 7)', () => {
  const html = (device: string | null) =>
    renderToStaticMarkup(createElement(SignInActions, { device, busy: false, onItWasMe: () => {}, onSignOut: async () => {} }))

  test('has It was me, and Sign it out as design buttons, the dialog trigger red', () => {
    const out = html('Chrome on Windows')
    expect(out).toContain('It was me')
    expect(out).toContain('Sign it out')
    expect(out).toContain('a-btn danger')
    expect(out).not.toContain('<dialog')
  })

  test('the dialog words and the done line are the spec words', () => {
    expect(t('sign_out_device_title', { device: 'Chrome on Windows' })).toBe('Sign out "Chrome on Windows"?')
    expect(t('sign_out_device_body')).toBe('It can no longer open HQ until it signs in again.')
    expect(t('sign_out_device_act')).toBe('Sign out device')
    expect(t('sign_out_device_done', { device: 'Chrome on Windows' })).toBe('Chrome on Windows is signed out.')
    expect(t('sign_out_all_keep')).toBe('Keep everyone signed in')
  })

  test('the body reads the time from the item in the browser clock', () => {
    const at = new Date(2026, 10, 5, 14, 32).toISOString()
    expect(signInTime(at, new Date(2026, 10, 5, 18, 0))).toBe('14:32 today')
    expect(signInTime(at, new Date(2026, 10, 7, 9, 0))).toBe('14:32 on 5 Nov')
    const body = renderToStaticMarkup(createElement(SignInBody, { device: 'Chrome on Windows', createdAt: new Date().toISOString() }))
    expect(body).toContain('Chrome on Windows signed in with a one-time code at')
    expect(body).toContain('If it was not you, sign it out now.')
  })
})
