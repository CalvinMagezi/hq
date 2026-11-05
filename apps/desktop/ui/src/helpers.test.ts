import { describe, expect, test } from 'bun:test'
import { afterTry, grouped, minutesLeft, nothingFound, orderRows, rowAction, rowLine, shortHome, tone, type HelperView } from './helpers'

const v = (over: Partial<HelperView>): HelperView => ({
  id: 'x',
  name: 'X',
  group: 'coding',
  path: '/Users/me/.local/bin/x',
  state: 'ready',
  version: '1.0.0',
  version_from_package: true,
  latest: null,
  new_here: false,
  can_connect: false,
  connected: false,
  can_sign_in: false,
  install: null,
  ...over,
})

describe('the Helpers tab rows', () => {
  test('rows that need you come first, then ready, then not installed', () => {
    const rows = [
      v({ id: 'a', state: 'not_installed' }),
      v({ id: 'b', state: 'ready' }),
      v({ id: 'c', state: 'needs_signin' }),
      v({ id: 'd', state: 'ready' }),
    ]
    expect(orderRows(rows).map((r) => r.id)).toEqual(['c', 'b', 'd', 'a'])
  })

  test('each state has its line and its one action', () => {
    expect(rowLine(v({ state: 'not_connected', can_connect: true }))).toBe('1.0.0 · not connected to HQ yet')
    expect(rowAction(v({ state: 'not_connected', can_connect: true }))).toBe('connect')
    expect(rowLine(v({ state: 'update_available', version: '0.58.0', latest: '0.60.1' }))).toBe('0.58.0 · 0.60.1 is out')
    expect(rowLine(v({ state: 'ready', connected: true, can_connect: true }))).toBe('1.0.0 · connected to HQ')
    expect(rowLine(v({ state: 'not_installed' }))).toBe('Not on this computer')
    expect(rowAction(v({ state: 'not_installed' }))).toBe('install')
    expect(rowAction(v({ state: 'program_not_found' }))).toBe('choose')
    expect(rowAction(v({ state: 'check_failed' }))).toBe('check_again')
    expect(rowAction(v({ state: 'needs_you' }))).toBe('see_asked')
  })

  test('an app has no Try it, and a program new here shows its full path once', () => {
    expect(rowAction(v({ group: 'app', state: 'ready' }))).toBeNull()
    expect(rowLine(v({ state: 'ready' }), true)).toContain('new here: /Users/me/.local/bin/x')
  })

  test('colours follow the rules: amber needs you, blue works, grey is fine', () => {
    expect(tone('needs_you')).toBe('amber')
    expect(tone('checking')).toBe('blue')
    expect(tone('ready')).toBe('grey')
    expect(tone('update_available')).toBe('grey')
  })

  test('the three groups hold nine, two and one', () => {
    const rows = [
      ...Array.from({ length: 9 }, (_, i) => v({ id: `c${i}` })),
      v({ id: 'h', group: 'general' }),
      v({ id: 'o', group: 'general' }),
      v({ id: 'd', group: 'app' }),
    ]
    const g = grouped(rows)
    expect([g.coding.length, g.general.length, g.app.length]).toEqual([9, 2, 1])
  })

  test('no helpers at all is told apart from some', () => {
    expect(nothingFound([v({ state: 'not_installed' }), v({ group: 'app', state: 'ready' })])).toBe(true)
    expect(nothingFound([v({ state: 'ready' })])).toBe(false)
  })

  test('a sign-in link on the screen means it needs a sign-in', () => {
    expect(afterTry({ state: 'trying', link_ready: true })).toBe('needs_signin')
    expect(afterTry({ state: 'needs_you', link_ready: false })).toBe('needs_you')
    expect(afterTry({ state: 'trying', link_ready: false })).toBe('ready')
  })

  test('the sign-in countdown and the home shortening', () => {
    expect(minutesLeft(0, 48_000)).toBe('9:12')
    expect(minutesLeft(0, 900_000)).toBe('0:00')
    expect(shortHome('/Users/me/.local/bin/claude', '/Users/me')).toBe('~/.local/bin/claude')
  })
})
