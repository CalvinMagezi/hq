import { describe, expect, test } from 'bun:test'
import { EMBED_HEAD_SCRIPT, isEmbedRequest } from './embedHost'

/** Runs the head script against a fake page and returns what it left behind. */
function runHeadScript(search: string, stored: Record<string, string> = {}) {
  const store = { ...stored }
  const attrs: Record<string, string> = {}
  const fake = {
    URLSearchParams,
    location: { search },
    sessionStorage: { getItem: (k: string) => store[k] ?? null, setItem: (k: string, v: string) => void (store[k] = v) },
    document: { documentElement: { setAttribute: (k: string, v: string) => void (attrs[k] = v) } },
  }
  new Function('URLSearchParams', 'location', 'sessionStorage', 'document', EMBED_HEAD_SCRIPT)(
    fake.URLSearchParams,
    fake.location,
    fake.sessionStorage,
    fake.document,
  )
  return { store, attrs }
}

describe('isEmbedRequest', () => {
  test('?embed=1 turns it on, and so does the old ?host=android', () => {
    expect(isEmbedRequest('?embed=1', null)).toBe(true)
    expect(isEmbedRequest('?view=board&embed=1', null)).toBe(true)
    expect(isEmbedRequest('?host=android', null)).toBe(true)
    expect(isEmbedRequest('?view=board&host=android', null)).toBe(true)
  })

  test('an earlier page in the tab keeps it on after the query is gone', () => {
    expect(isEmbedRequest('', '1')).toBe(true)
  })

  test('anything else is the plain web app', () => {
    expect(isEmbedRequest('', null)).toBe(false)
    expect(isEmbedRequest('?host=ios', null)).toBe(false)
    expect(isEmbedRequest('?embed=0', null)).toBe(false)
    expect(isEmbedRequest('?x=embed=1', null)).toBe(false)
  })
})

describe('the head script (check 1)', () => {
  test('?embed=1 sets the flag and the attribute before first paint', () => {
    const { store, attrs } = runHeadScript('?embed=1')
    expect(store['hq-embed']).toBe('1')
    expect(attrs['data-embed']).toBe('1')
  })

  test('?host=android sets the same one flag', () => {
    const { store, attrs } = runHeadScript('?host=android')
    expect(store['hq-embed']).toBe('1')
    expect(attrs['data-embed']).toBe('1')
  })

  test('a tab opened embedded stays embedded without the query', () => {
    expect(runHeadScript('', { 'hq-embed': '1' }).attrs['data-embed']).toBe('1')
  })

  test('a plain page sets nothing', () => {
    const { store, attrs } = runHeadScript('')
    expect(store['hq-embed']).toBeUndefined()
    expect(attrs['data-embed']).toBeUndefined()
  })
})
