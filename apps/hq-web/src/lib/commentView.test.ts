import { describe, expect, test } from 'bun:test'
import DOMPurify from 'dompurify'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MarkdownViewer, markdownReady } from '../components/MarkdownViewer'
import { commentMarker, countComments, foldComments, pickAnchor, splitComments } from './commentView'
import type { SuiteBlock, SuiteComment } from './suiteApi'

const NOTE = [
  '---',
  'suite_kind: doc',
  '---',
  '# Report',
  '',
  'Revenue grew by twelve percent.',
  '',
  '```comment on=0123456789ab by="Sam" at=2026-11-20 state=open',
  'Check this figure.',
  '```',
  '',
  '```comment on=0123456789ab at=2026-11-21 state=resolved',
  'Done, it is right.',
  '```',
  '',
  'Costs fell.',
  '',
  '```comment on=ba9876543210',
  'No state means open.',
  '```',
  '',
].join('\n')

const block = (index: number, kind: string, src: string): SuiteBlock => ({ index, hash: `${String(index).padStart(12, '0')}`, kind, src })
const BLOCKS = [block(0, 'heading 1', '# Report'), block(1, 'paragraph', 'Revenue grew by\ntwelve percent.'), block(2, 'comment', '```comment on=x\nhi\n```'), block(3, 'paragraph', 'Costs fell.'), block(4, 'sheet', '```sheet name=A\n```'), block(5, 'sheet', '```sheet name=B\n```')]

describe('counting', () => {
  test('open and resolved comments come from the blocks in the text, a missing state meaning open', () => {
    expect(countComments(NOTE)).toEqual({ open: 2, resolved: 1 })
    expect(countComments('# nothing here')).toEqual({ open: 0, resolved: 0 })
  })
  test('a comment shown as code inside a longer fence is not counted', () => {
    expect(countComments('````\n```comment on=0123456789ab\nx\n```\n````\n')).toEqual({ open: 0, resolved: 0 })
  })
  test('open comments come before resolved ones, each in the order written', () => {
    const row = (index: number, state: string): SuiteComment => ({ index, hash: 'h', on: 'o', by: null, at: null, state, text: 't', orphaned: false, anchor_preview: null })
    const { open, resolved } = splitComments([row(1, 'resolved'), row(2, 'open'), row(3, 'open')])
    expect(open.map((c) => c.index)).toEqual([2, 3])
    expect(resolved.map((c) => c.index)).toEqual([1])
  })
})

describe('where a new comment goes', () => {
  test('selected words find the block that holds them, a block that is exactly them first', () => {
    expect(pickAnchor(BLOCKS, { text: 'twelve percent' })?.index).toBe(1)
    expect(pickAnchor(BLOCKS, { text: 'Costs fell.' })?.index).toBe(3)
    expect(pickAnchor(BLOCKS, { text: '## Nothing like this' })).toBeNull()
  })
  test('the nth block of a kind, and never a comment', () => {
    expect(pickAnchor(BLOCKS, { kind: 'sheet', nth: 1 })?.index).toBe(5)
    expect(pickAnchor(BLOCKS, { kind: 'comment' })).toBeNull()
    expect(pickAnchor(BLOCKS, { text: 'on=x' })).toBeNull()
  })
  test('with no hint there is no choice made for the person', () => {
    expect(pickAnchor(BLOCKS)).toBeNull()
    expect(pickAnchor(BLOCKS, {})).toBeNull()
  })
})

describe('keeping comments out of the page', () => {
  test('a run of comment blocks folds into one marker that says how many, and how many are open', () => {
    const tokens = [
      { type: 'paragraph' },
      { type: 'code', lang: 'comment', raw: '```comment on=0123456789ab state=open\nA\n```\n' },
      { type: 'space' },
      { type: 'code', lang: 'comment', raw: '```comment on=0123456789ab state=resolved\nB\n```\n' },
      { type: 'paragraph' },
      { type: 'code', lang: 'comment', raw: '```comment on=ba9876543210\nC\n```\n' },
    ]
    const folded = foldComments(tokens as { type: string; lang?: string; raw?: string }[]) as { type: string; hqComments?: number; hqOpen?: number }[]
    expect(folded.map((t) => t.type)).toEqual(['paragraph', 'code', 'paragraph', 'code'])
    expect([folded[1].hqComments, folded[1].hqOpen]).toEqual([2, 1])
    expect([folded[3].hqComments, folded[3].hqOpen]).toEqual([1, 1])
  })
  test('code that is not a comment is left alone', () => {
    const tokens = [{ type: 'code', lang: 'ts', raw: '```ts\nx\n```' }]
    expect(foldComments(tokens)).toEqual(tokens)
  })
  test('the marker is escaped and marks an open run', () => {
    expect(commentMarker(1, '2 <comments>')).toContain('2 &lt;comments&gt;')
    expect(commentMarker(1, 'x')).toContain('is-open')
    expect(commentMarker(0, 'x')).not.toContain('is-open')
  })
  test('the reading viewer draws no comment text, only a marker after the block', async () => {
    await markdownReady()
    const sanitize = DOMPurify.sanitize
    DOMPurify.sanitize = ((dirty: string) => dirty) as typeof DOMPurify.sanitize
    try {
      const html = renderToStaticMarkup(createElement(MarkdownViewer, { content: NOTE, bare: true }))
      expect(html).toContain('Revenue grew by twelve percent.')
      expect(html).not.toContain('Check this figure.')
      expect(html).not.toContain('code-block')
      expect(html.match(/md-cmt-mark/g)).toHaveLength(2)
      expect(html).toContain('2 comments')
      expect(html).toContain('1 comment<')
    } finally {
      DOMPurify.sanitize = sanitize
    }
  })
})
