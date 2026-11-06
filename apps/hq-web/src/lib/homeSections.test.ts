import { expect, test } from 'bun:test'
import { renderToString } from 'react-dom/server'
import { createElement } from 'react'
import { sectionState } from './homeSections'
import { HomeSection } from '~/components/vault/HomeSection'
import { HomeSections } from '~/components/vault/HomeSections'

test('sectionState covers loading, empty, error and ready', () => {
  const base = { data: undefined, isPending: false, isError: false, error: null }
  expect(sectionState({ ...base, isPending: true })).toEqual({ kind: 'loading' })
  expect(sectionState({ ...base, data: [] })).toEqual({ kind: 'empty' })
  expect(sectionState({ ...base, isError: true, error: new Error('boom') })).toEqual({ kind: 'error', message: 'boom' })
  expect(sectionState({ ...base, isError: true, error: 'x' })).toEqual({ kind: 'error', message: 'Request failed' })
  expect(sectionState({ ...base, data: [1, 2] })).toEqual({ kind: 'ready', items: [1, 2] })
})

test('saved data stays visible when a refresh fails', () => {
  const s = sectionState({ data: [1], isPending: false, isError: true, error: new Error('offline') })
  expect(s.kind).toBe('ready')
})

const html = (state: Parameters<typeof HomeSection>[0]['state']) =>
  renderToString(
    createElement(HomeSection, { title: 'Pinned', state, emptyText: 'Nothing pinned yet.', failedKey: 'fail_load_pinned_no_reply', onRetry: () => {}, render: () => null }),
  )

test('HomeSection renders loading, empty and error states', () => {
  expect(html({ kind: 'loading' })).toContain('a-skel')
  expect(html({ kind: 'empty' })).toContain('Nothing pinned yet.')
  const err = html({ kind: 'error', message: 'offline' })
  // The shell is not Connected here, so the status line speaks and the section stays quiet.
  expect(err).not.toContain('Could not load')
  expect(err).not.toContain('Retry')
})

test('HomeSections orders pinned then recent, and an empty section keeps its siblings', () => {
  const noop = () => {}
  const out = renderToString(
    createElement(HomeSections, {
      pinned: { kind: 'empty' },
      recent: { kind: 'loading' },
      onRetryPinned: noop,
      onRetryRecent: noop,
    }),
  )
  const at = (t: string) => out.indexOf(`data-home-section="${t}"`)
  expect(at('Pinned')).toBeGreaterThanOrEqual(0)
  expect(at('Pinned')).toBeLessThan(at('Recent'))
  expect(out).toContain('Nothing is pinned yet')
  expect(out).not.toContain('Could not load')
  expect(out).not.toContain('In Progress Tasks')
})
