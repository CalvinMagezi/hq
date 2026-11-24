import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { CommentItem, CommentsButton, RefusedItem } from './CommentsPanel'
import type { ShownComment } from '~/lib/commentOutbox'

const base: ShownComment = { index: -1, hash: '', on: 'b1', by: 'Sam', at: '2026-11-21', state: 'open', text: 'Check this', orphaned: false, anchor_preview: 'First line' }

describe('a comment waiting to be sent', () => {
  test('is drawn at once, marked as waiting, with a way to take it back', () => {
    const html = renderToStaticMarkup(createElement(CommentItem, { c: { ...base, waitingId: 'w1' }, onDiscard: () => undefined, disabled: false }))
    expect(html).toContain('Check this')
    expect(html).toContain('is-waiting')
    expect(html).toContain('Waiting to send')
    expect(html).toContain('>Discard<')
    expect(html).not.toContain('>Resolve<')
  })
  test('a resolve that waits says so and offers no second resolve', () => {
    const html = renderToStaticMarkup(createElement(CommentItem, { c: { ...base, index: 3, resolving: true }, disabled: false }))
    expect(html).toContain('Resolved here. Waiting to send.')
    expect(html).not.toContain('>Resolve<')
  })
  test('one HQ has is plain and can be resolved', () => {
    const html = renderToStaticMarkup(createElement(CommentItem, { c: { ...base, index: 3 }, onResolve: () => undefined, disabled: false }))
    expect(html).not.toContain('is-waiting')
    expect(html).toContain('>Resolve<')
  })
})

describe('a comment HQ refused after a wait', () => {
  const refused = (op: object, reason = 'The block you commented on has changed.') => renderToStaticMarkup(createElement(RefusedItem, { r: { clientId: 'x', op: op as never, reason }, onDiscard: () => undefined }))
  test('shows the sentence and the persons words, with Copy text and Discard', () => {
    const html = refused({ kind: 'comment-add', path: 'N/a.md', on: 'b1', by: 'Sam', text: 'my words', at: '2026-11-21' })
    expect(html).toContain('role="alert"')
    expect(html).toContain('could not be added: The block you commented on has changed.')
    expect(html).toContain('my words')
    expect(html).toContain('>Copy text<')
    expect(html).toContain('>Discard<')
  })
  test('a refused resolve says so and can be discarded', () => {
    const html = refused({ kind: 'comment-resolve', path: 'N/a.md', index: 3, hash: 'c1' })
    expect(html).toContain('could not be resolved')
    expect(html).toContain('>Discard<')
    expect(html).not.toContain('Copy text')
  })
})

describe('the Comments button', () => {
  test('still renders on its own, without waiting for the saved copy', () => {
    const html = renderToStaticMarkup(createElement(CommentsButton, { path: 'N/a.md', text: 'x', canEdit: true, onChanged: () => undefined }))
    expect(html).toContain('>Comments<')
    expect(html).not.toContain('su-badge-warn')
  })
})
