import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { PLACES, ROUTE_TABLE, legacyRedirect, liteRedirect, phoneLink, placeOf, startPlace, targetHref } from './routes'
import { placesFor, toEdition } from './edition'
import { t } from '~/strings'

const go = (pathname: string, search = '', hash = '', ctx = {}) => {
  const r = legacyRedirect({ pathname, search, hash }, ctx)
  return r && targetHref(r)
}

describe('route table (spec 01 checks 1 and 2)', () => {
  test('every old address redirects to the row that lists it', () => {
    for (const row of ROUTE_TABLE) {
      for (const old of row.replaces.filter((o) => !o.startsWith('/notes/'))) {
        const [path, query = ''] = old.replace('{id}', 'x1').split('?')
        const [p, h = ''] = path.split('#')
        const got = go(p, query ? `?${query}` : '', h)
        expect(got).not.toBeNull()
        expect(got!.split('#')[0].split('?')[0]).toBe(row.web.replace(':id', 'x1').split('?')[0])
      }
    }
  })
  test('no phone link carries a search query', () => {
    for (const row of ROUTE_TABLE) expect(row.phone ?? '').not.toContain('?q=')
  })
  test('notes ids pair with hq://notes/n/{id}', () => {
    expect(ROUTE_TABLE.find((r) => r.web === '/notes/n/:id')?.phone).toBe('hq://notes/n/{id}')
  })
  test('phoneOnly rows are flagged', () => {
    expect(ROUTE_TABLE.some((r) => r.phoneOnly)).toBe(true)
  })
})

describe('old addresses keep the item and the fragment', () => {
  test.each([
    ['/sessions', '?id=h-31', '', '/helpers/h-31'],
    ['/chat', '?thread=abc', '#x', '/chat/abc#x'],
    ['/tasks', '?task=T-12', '', '/tasks/T-12'],
    ['/vault/Projects/HQ/Plan.md', '', '', '/notes/Projects/HQ/Plan.md'],
    ['/vault', '', '', '/notes'],
    ['/notifications', '', '', '/inbox'],
    ['/usage', '', '', '/settings/spending'],
    ['/settings/assistant', '', '', '/settings'],
    ['/settings/speech', '', '', '/settings'],
    ['/settings/bogus', '', '', '/settings'],
  ])('%s%s%s', (path, search, hash, want) => {
    expect(go(path, search, hash)).toBe(want)
  })
  test('current addresses stay put', () => {
    for (const p of ['/chat', '/notes', '/inbox', '/helpers', '/tasks', '/settings/browser']) expect(go(p)).toBeNull()
  })
})

describe('the start place', () => {
  test('/ is Chat, and Notes on Lite', () => {
    expect(go('/')).toBe('/chat')
    expect(go('/', '', '', { edition: 'full' })).toBe('/chat')
    expect(go('/', '', '', { edition: 'lite' })).toBe('/notes')
    expect(startPlace('lite')).toBe('/notes')
  })
  test('a login link opens Chat, not Notes', () => {
    expect(go('/vault', '', '', { signInLink: true })).toBe('/chat')
  })
})

describe('Lite (spec 02)', () => {
  test('Chat and Helpers open Notes with a line', () => {
    expect(liteRedirect('/chat/abc')?.target.to).toBe('/notes')
    expect(liteRedirect('/helpers')?.notice).toEqual({ key: 'lite_no_place', place: 'helpers' })
    expect(t('lite_no_place', { place: t('place_chat') })).toBe('This HQ is HQ Lite. It has no Chat.')
  })
  test('Spending opens This browser', () => {
    expect(liteRedirect('/settings/spending')?.target.to).toBe('/settings/browser')
  })
  test('Notes, Tasks and Inbox are left alone', () => {
    for (const p of ['/notes', '/notes/a/b', '/tasks', '/inbox']) expect(liteRedirect(p)).toBeNull()
  })
  test('the rail has exactly three places', () => {
    expect(placesFor('lite').map((p) => p.id)).toEqual(['notes', 'tasks', 'inbox'])
    expect(placesFor('full').map((p) => p.id)).toEqual(['notes', 'tasks', 'chat', 'helpers', 'inbox'])
    expect(placesFor(null).length).toBe(3)
  })
  test('toEdition only trusts known names', () => {
    expect(toEdition('lite')).toBe('lite')
  })
})

describe('places', () => {
  test('a sub-path lights its place', () => {
    expect(placeOf('/notes/Projects/HQ/Plan.md')).toBe('notes')
    expect(placeOf('/helpers/h-1')).toBe('helpers')
    expect(placeOf('/settings')).toBeNull()
    expect(PLACES.length).toBe(5)
  })
  test('the phone link follows the place', () => {
    expect(phoneLink('/notes')).toBe('hq://notes')
  })
})

describe('manifest (spec 01)', () => {
  const m = JSON.parse(readFileSync(join(import.meta.dir, '../../public/manifest.json'), 'utf8'))
  test('names and colours', () => {
    expect(m.name).toBe('AgentHQ')
    expect(m.short_name).toBe('AgentHQ')
    expect(m.theme_color).toBe('#000000')
  })
  test('exactly three shortcuts to current addresses', () => {
    expect(m.shortcuts.map((s: { url: string }) => s.url)).toEqual(['/chat', '/notes', '/inbox'])
  })
})
