import { describe, expect, test } from 'bun:test'
import { EMBED_HEAD_SCRIPT, isEmbedRequest } from './embedHost'

describe('isEmbedRequest', () => {
  test('the query string turns it on', () => {
    expect(isEmbedRequest('?host=android', null)).toBe(true)
    expect(isEmbedRequest('?view=board&host=android', null)).toBe(true)
  })

  test('an earlier page in the tab keeps it on after the query is gone', () => {
    expect(isEmbedRequest('', 'android')).toBe(true)
  })

  test('anything else is the plain web app', () => {
    expect(isEmbedRequest('', null)).toBe(false)
    expect(isEmbedRequest('?host=ios', null)).toBe(false)
    expect(isEmbedRequest('?host=android', 'other')).toBe(true)
    expect(isEmbedRequest('?x=host=android', null)).toBe(false)
  })

  test('the head script uses the same names as the module', () => {
    expect(EMBED_HEAD_SCRIPT).toContain("'host'")
    expect(EMBED_HEAD_SCRIPT).toContain("'hq-embed-host'")
    expect(EMBED_HEAD_SCRIPT).toContain("'android'")
  })
})
