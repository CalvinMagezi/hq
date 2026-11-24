import { afterEach, expect, test } from 'bun:test'
import { commentAdd, commentResolve, commentsIn, fetchComments } from './suiteApi'

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

/** Every fetch answers with `body`; returns what was sent. */
function serverAnswers(body: unknown, status = 200) {
  const sent: { url: string; body: Record<string, unknown> }[] = []
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    sent.push({ url, body: JSON.parse(String(init.body)) })
    return new Response(JSON.stringify(body), { status })
  }) as unknown as typeof fetch
  return sent
}

const ROW = { index: 3, hash: 'aaaaaaaaaaaa', on: 'bbbbbbbbbbbb', by: 'Sam', at: '2026-11-20', state: 'open', text: 'Check this.', orphaned: false, anchor_preview: '## Findings' }

test('reading the comments asks the edit route for the `comments` action and returns the list as sent', async () => {
  const sent = serverAnswers({ revision: 'r1', comments: [ROW] })
  const got = await fetchComments('Notebooks/D.md')
  expect(sent[0].url).toBe('/api/suite/edit')
  expect(sent[0].body).toEqual({ path: 'Notebooks/D.md', action: 'comments' })
  expect(got.comments[0].anchor_preview).toBe('## Findings')
})

test('adding a comment sends the block hash and the text, and leaves out what was not given', async () => {
  const sent = serverAnswers({ summary: 'Added a comment', revision: 'r2', changed: true, comments: [ROW] })
  const r = await commentAdd('Notebooks/D.md', { on: 'bbbbbbbbbbbb', text: 'Check this.', by: 'Sam' })
  expect(sent[0].body).toEqual({ path: 'Notebooks/D.md', action: 'add_comment', on: 'bbbbbbbbbbbb', text: 'Check this.', by: 'Sam' })
  expect(commentsIn(r)?.comments).toHaveLength(1)
})

test('resolving quotes the comment index and its hash', async () => {
  const sent = serverAnswers({ summary: 'Resolved comment 3', revision: 'r3', comments: [{ ...ROW, state: 'resolved' }] })
  const r = await commentResolve('Notebooks/D.md', { index: 3, hash: 'aaaaaaaaaaaa' })
  expect(sent[0].body).toEqual({ path: 'Notebooks/D.md', action: 'resolve_comment', index: 3, hash: 'aaaaaaaaaaaa' })
  expect(commentsIn(r)?.comments[0].state).toBe('resolved')
})

test("a refusal rejects with hq's sentence, and a dry run is passed on", async () => {
  serverAnswers({ error: 'that block changed since it was read, so the comment was not added' }, 422)
  await expect(commentAdd('Notebooks/D.md', { on: 'bbbbbbbbbbbb', text: 'x' })).rejects.toThrow('changed since it was read')
  const sent = serverAnswers({ summary: 's', revision: 'r', preview: true })
  await commentAdd('Notebooks/D.md', { on: 'bbbbbbbbbbbb', text: 'x' }, true)
  expect(sent[0].body.dry_run).toBe(true)
})

test('an answer without a list gives none', () => {
  expect(commentsIn({ revision: 'r' })).toBeNull()
})
