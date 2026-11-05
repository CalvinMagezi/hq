import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { checkSetupKey, keyAnswer, saveSetupKey, showsNoSignIn, KEY_ANSWER_STRING } from './setupApi'
import { isCover, needsSignInPage, ownsSignedOut } from './covers'
import { SetupForm, SetupNoSignIn } from '~/routes/setup'
import { ConnectNotice, ConsentForm, declinesOnKey } from '~/routes/connect'
import { SignedOutView, SignInPrompt, signedOutTitle } from '~/components/SignInPrompt'
import { t } from '~/strings'

const src = (p: string) => readFileSync(join(import.meta.dir, '..', p), 'utf8')
const esc = (s: string) => s.replace(/'/g, '&#x27;')
const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

function mockFetch(json: unknown) {
  const urls: string[] = []
  globalThis.fetch = (async (url: string) => {
    urls.push(String(url))
    return new Response(JSON.stringify(json), { status: 200, headers: { 'content-type': 'application/json' } })
  }) as unknown as typeof fetch
  return urls
}

const form = {
  provider: 'open_router' as const, apiKey: 'k', checking: false, saving: false, check: null, saveError: false,
  onProvider: () => {}, onKey: () => {}, onCheck: () => {}, onOpen: () => {},
}

describe('/setup (check 1, 2)', () => {
  test('has Check key and Open HQ as separate buttons, and the old one-button words are gone', () => {
    const html = renderToStaticMarkup(createElement(SetupForm, form))
    expect(html).toContain(t('setup_check'))
    expect(html).toContain(t('setup_open'))
    expect(html).toContain(t('setup_continue_unchecked'))
    expect(html).not.toContain('Test connection and open HQ')
    expect(html).not.toContain('Base URL')
  })
  test('Check key only asks the service; Open HQ only saves', async () => {
    const urls = mockFetch({ ok: true })
    expect(await checkSetupKey('open_router', 'k')).toEqual({ accepted: true })
    expect(urls).toEqual(['/api/setup/test'])
    urls.length = 0
    await saveSetupKey('open_router', 'k')
    expect(urls).toEqual(['/api/setup/provider'])
    expect(src('routes/setup.tsx')).not.toContain('testSetupKey')
  })
  test('a refused key shows plain words, never a status code', async () => {
    mockFetch({ ok: false, error: 'The service answered 401.' })
    const check = await checkSetupKey('anthropic', 'k')
    expect(check).toEqual({ accepted: false, answer: 'refused' })
    const html = renderToStaticMarkup(createElement(SetupForm, { ...form, check }))
    expect(html).toContain(t('setup_refused'))
    expect(html).not.toContain('401')
  })
  test('other answers map to credit, busy and silence', () => {
    expect(keyAnswer('402 insufficient credits')).toBe('no_credit')
    expect(keyAnswer('429 rate limit')).toBe('too_busy')
    expect(keyAnswer('timed out')).toBe('no_answer')
    for (const s of Object.values(KEY_ANSWER_STRING)) expect(t(s)).not.toMatch(/\d{3}/)
  })
  test('an accepted key says so under the button', () => {
    expect(renderToStaticMarkup(createElement(SetupForm, { ...form, check: { accepted: true } }))).toContain(t('setup_accepted'))
  })
  test('a server with no web sign-in shows hq env set openrouter', () => {
    expect(showsNoSignIn({ needs_setup: true, available: false })).toBe(true)
    expect(showsNoSignIn({ needs_setup: true, available: true })).toBe(false)
    const html = renderToStaticMarkup(createElement(SetupNoSignIn))
    expect(html).toContain('hq env set openrouter')
    expect(html).toContain(esc(t('setup_no_signin_title')))
    expect(src('routes/setup.tsx')).not.toContain('OPENROUTER_API_KEY')
  })
  test('the key field is a password field a person can show', () => {
    const html = renderToStaticMarkup(createElement(SetupForm, form))
    expect(html).toContain('type="password"')
    expect(html).toContain('autoComplete="off"')
  })
})

const req = { client_name: 'Claude', client_id: 'c', redirect_host: 'claude.ai', resource: 'r', created_at: 1_700_000_000, expires_at: 1_700_000_600, armed_until: 1_700_000_900, default_scope: 'read_only' as const }
const consent = { req, scope: 'read_only' as const, label: 'Claude', busy: false, error: null, onScope: () => {}, onLabel: () => {}, onAllow: () => {}, onDecline: () => {} }

describe('/connect (check 3, 6)', () => {
  test('Decline sits left of Allow and the old words are gone', () => {
    const html = renderToStaticMarkup(createElement(ConsentForm, consent))
    expect(html.indexOf(t('connect_decline'))).toBeGreaterThan(-1)
    expect(html.indexOf(t('connect_decline'))).toBeLessThan(html.lastIndexOf(t('connect_allow')))
    expect(html).not.toMatch(/Deny|Approve/)
  })
  test('Esc means Decline, and nothing answers while busy', () => {
    expect(declinesOnKey('Escape', false)).toBe(true)
    expect(declinesOnKey('Escape', true)).toBe(false)
    expect(declinesOnKey('Enter', false)).toBe(false)
  })
  test('the chosen level has an ink edge and Read only is the default', () => {
    const html = renderToStaticMarkup(createElement(ConsentForm, consent))
    expect(html).toContain('aria-checked="true"')
    expect(html.match(/border-color:var\(--ink\)/g)?.length).toBe(1)
  })
  test('no text outside Technical details says connector, and Claude on the web is named', () => {
    const all = [
      renderToStaticMarkup(createElement(ConsentForm, consent)),
      t('connect_closed_title'),
      t('connect_closed_line'),
      t('connect_closed_open'),
      ...(['none', 'loading', 'expired', 'signed_out'] as const).map((k) => renderToStaticMarkup(createElement(ConnectNotice, { kind: k }))),
    ].join(' ')
    expect(all.toLowerCase()).not.toContain('connector')
    expect(all).toContain('Claude on the web')
    expect(t('connect_closed_line')).toContain('Claude on the web')
  })
})

describe('signed out (check 4, 5)', () => {
  test('the empty view names the place and never says unauthorized', () => {
    const html = renderToStaticMarkup(createElement(SignedOutView, { pathname: '/notes', embedded: false }))
    expect(html).toContain(t('signed_out_notes'))
    expect(html).toContain(esc(t('signed_out_line')))
    expect(html).not.toMatch(/unauthorized|Retry/i)
    expect(html).not.toContain('role="alert"')
    expect(signedOutTitle('/tasks/x')).toBe(t('signed_out_tasks'))
  })
  test('the sheet is not a banner over every screen', () => {
    expect(renderToStaticMarkup(createElement(SignInPrompt))).toBe('')
    expect(src('components/SignInPrompt.tsx')).not.toContain('borderBottom')
  })
  test('a browser with nothing stored goes to the full page; one signed out keeps the shell', () => {
    expect(needsSignInPage({ signInNeeded: true, hadSignIn: false })).toBe(true)
    expect(needsSignInPage({ signInNeeded: true, hadSignIn: true })).toBe(false)
    expect(needsSignInPage({ signInNeeded: false, hadSignIn: false })).toBe(false)
  })
  test('setup, connect and signin are covers without the shell', () => {
    for (const p of ['/setup', '/connect', '/signin']) expect(isCover(p)).toBe(true)
    expect(isCover('/notes')).toBe(false)
    expect(ownsSignedOut('/connect')).toBe(true)
    expect(ownsSignedOut('/setup')).toBe(false)
    expect(src('routes/__root.tsx')).toContain('needsSignInPage')
    expect(src('routes/signin.tsx')).toContain('SignInPage')
  })
})
