import { describe, expect, test } from 'bun:test'
import DOMPurify from 'dompurify'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MarkdownViewer, markdownReady } from '../MarkdownViewer'
import { prependedTop } from './MessageList'
import { letsGo } from './useStickToBottom'

describe('a chat keeps the reader\'s place', () => {
  test('a saved reply has its text on first paint, so the chat does not collapse when a reply finishes', async () => {
    await markdownReady()
    // The test runner has no DOM for the sanitizer to work in.
    const sanitize = DOMPurify.sanitize
    DOMPurify.sanitize = ((dirty: string) => dirty) as typeof DOMPurify.sanitize
    try {
      const html = renderToStaticMarkup(createElement(MarkdownViewer, { content: 'Some **bold** words', bare: true }))
      expect(html).toContain('<strong>bold</strong>')
    } finally {
      DOMPurify.sanitize = sanitize
    }
  })

  test('only the reader moving up stops the follow', () => {
    expect(letsGo(false, 100, 500, true)).toBe(true)
    expect(letsGo(false, 100, 500, false)).toBe(false)
    expect(letsGo(true, 100, 500, true)).toBe(false)
    expect(letsGo(false, 500, 100, true)).toBe(false)
  })

  test('older messages added above leave the place alone with or without scroll anchoring', () => {
    expect(prependedTop(300, 300, 900, true)).toBe(1200)
    expect(prependedTop(1200, 300, 900, true)).toBe(1200)
    expect(prependedTop(340, 300, 900, false)).toBe(1240)
  })
})
