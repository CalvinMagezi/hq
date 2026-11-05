import { expect, test } from 'bun:test'
import { forgetSearch, readBack, rememberSearch, withQuery, withoutQuery } from './searchBack'

function fakeWindow(href: string) {
  const win = { history: { state: null as unknown, replaceState(state: unknown, _t: string, url: string) { this.state = state; const u = new URL(url, 'http://x.invalid'); win.location = { pathname: u.pathname, search: u.search, hash: u.hash } } }, location: { pathname: '', search: '', hash: '' } }
  const u = new URL(href, 'http://x.invalid')
  win.location = { pathname: u.pathname, search: u.search, hash: u.hash }
  return win
}

test('the address gets ?q= and drops it again, keeping other words in it', () => {
  expect(withQuery('/tasks?view=board', 'relay notes')).toBe('/tasks?view=board&q=relay+notes')
  expect(withoutQuery('/tasks?view=board&q=relay+notes')).toBe('/tasks?view=board')
})

test('Back to the entry Search was left from reopens it with the same words, scope, folder and scroll', () => {
  const win = fakeWindow('/notes')
  rememberSearch({ q: 'battery', scope: 'notes', folder: 'Projects', scroll: 240 }, win as never)
  expect(win.location.search).toBe('?q=battery')
  expect(readBack(win.location.search, win.history.state)).toEqual({ q: 'battery', scope: 'notes', folder: 'Projects', scroll: 240 })
})

test('it reopens once: after the marker is dropped a second Back does not', () => {
  const win = fakeWindow('/notes')
  rememberSearch({ q: 'battery', scope: 'all', folder: null, scroll: 0 }, win as never)
  forgetSearch(win as never)
  expect(win.location.search).toBe('')
  expect(readBack(win.location.search, win.history.state)).toBeNull()
})

test('an address with ?q= that Search did not leave from reopens nothing', () => {
  expect(readBack('?q=battery', null)).toBeNull()
  expect(readBack('?q=battery', { hqSearch: { q: 'other', scope: 'all', folder: null, scroll: 0 } })).toBeNull()
})
