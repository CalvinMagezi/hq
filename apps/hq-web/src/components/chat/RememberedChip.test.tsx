import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RememberedChip } from './RememberedChip'
import { toRecalled } from '~/lib/threadApi'
import { tn } from '~/strings'

const draw = (ids: number[]) => renderToStaticMarkup(createElement(RememberedChip, { ids }))

describe('RememberedChip', () => {
  test('says how many notes were remembered, singular and plural', () => {
    expect(draw([4])).toContain(tn('chat_remembered', 1))
    expect(draw([4, 9])).toContain(tn('chat_remembered', 2))
  })

  test('starts closed, so a reply does not open with a list of notes', () => {
    const out = draw([4, 9])
    expect(out).toContain('aria-expanded="false"')
    expect(out).not.toContain('remembered-list')
  })

  test('draws nothing when nothing was remembered', () => {
    expect(draw([])).toBe('')
  })
})

describe('toRecalled', () => {
  test('keeps whole positive numbers and drops anything else', () => {
    expect(toRecalled([3, 5])).toEqual([3, 5])
    expect(toRecalled([3, 'x', 1.5, -2, null, 0])).toEqual([3])
  })

  test('gives nothing for a missing or malformed value', () => {
    expect(toRecalled(undefined)).toBeUndefined()
    expect(toRecalled('3')).toBeUndefined()
    expect(toRecalled([])).toBeUndefined()
  })
})
