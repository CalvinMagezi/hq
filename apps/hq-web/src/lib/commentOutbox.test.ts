import { describe, expect, test } from 'bun:test'
import { createOutbox, type OutboxEntry, type OutboxStore } from './outbox'
import { HqHttpError } from './hqAuth'
import type { SuiteComment } from './suiteApi'
import { discardCommentOp, mergeQueued, queueCommentOp, refusedComments, replayComments, today, waitingComments, type CommentApi } from './commentOutbox'

function memoryStore(): OutboxStore {
  const m = new Map<string, OutboxEntry[]>()
  return {
    async load(id) {
      return [...(m.get(id) ?? [])]
    },
    async save(id, e) {
      m.set(id, [...e])
    },
  }
}
const HQ = 'hq-a'
const P = 'N/a.md'
let tick = 0
const now = () => ++tick
const cm = (over: Partial<SuiteComment>): SuiteComment => ({ index: 3, hash: 'c1', on: 'b1', by: 'Sam', at: '2026-11-20', state: 'open', text: 'Check', orphaned: false, anchor_preview: null, ...over })

/** An HQ that keeps comments in memory; `fail` returns an error to throw for a call, or nothing. */
function fakeHq(start: SuiteComment[] = [], fail?: (call: string) => unknown) {
  const comments = [...start]
  const calls: string[] = []
  const api: CommentApi = {
    async list() {
      calls.push('list')
      const e = fail?.('list')
      if (e) throw e
      return { revision: 'r', comments: [...comments] }
    },
    async add(_p, a) {
      calls.push(`add:${a.text}`)
      const e = fail?.('add')
      if (e) throw e
      comments.push(cm({ index: comments.length * 2, hash: `h${comments.length}`, on: a.on, by: a.by ?? null, at: a.at ?? null, text: a.text }))
      return {}
    },
    async resolve(_p, a) {
      calls.push(`resolve:${a.index}`)
      const e = fail?.('resolve')
      if (e) throw e
      const c = comments.find((x) => x.index === a.index)
      if (c) c.state = 'resolved'
      return {}
    },
  }
  return { api, calls, comments }
}
const add = (text: string, over = {}) => ({ kind: 'comment-add' as const, path: P, on: 'b1', by: 'Sam', text, at: '2026-11-21', ...over })

describe('comments made with no connection', () => {
  test('are kept as small operations in the order they were made', async () => {
    const o = createOutbox(memoryStore())
    await queueCommentOp(o, HQ, add('one'), now)
    await queueCommentOp(o, HQ, { kind: 'comment-resolve', path: P, index: 3, hash: 'c1' }, now)
    await queueCommentOp(o, HQ, add('two', { path: 'N/b.md' }), now)
    const mine = await waitingComments(o, HQ, P)
    expect(mine.map((w) => w.op.kind)).toEqual(['comment-add', 'comment-resolve'])
    expect((await o.list(HQ)).every((e) => e.kind === 'comment')).toBe(true)
  })

  test('are sent oldest first, and the outbox empties', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([cm({ index: 3 })])
    await queueCommentOp(o, HQ, add('one'), now)
    await queueCommentOp(o, HQ, { kind: 'comment-resolve', path: P, index: 3, hash: 'c1' }, now)
    await queueCommentOp(o, HQ, add('two'), now)
    const r = await replayComments(o, HQ, hq.api)
    expect(hq.calls.filter((c) => c !== 'list')).toEqual(['add:one', 'resolve:3', 'add:two'])
    expect(r).toEqual({ sent: [P, P, P], refused: 0 })
    expect(await o.list(HQ)).toEqual([])
  })

  test('an add that already reached HQ is not added twice', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([cm({ index: 5, on: 'b1', by: 'Sam', at: '2026-11-21', text: 'one' })])
    await queueCommentOp(o, HQ, add('one'), now)
    await replayComments(o, HQ, hq.api)
    expect(hq.calls).toEqual(['list'])
    expect(hq.comments).toHaveLength(1)
    expect(await o.list(HQ)).toEqual([])
  })

  test('replayed twice after a flaky network, the comment is still added once', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq()
    // The first send is applied by HQ, but the page sees a network failure afterwards.
    const real = hq.api.add
    let lose = true
    hq.api.add = async (p, a) => {
      await real(p, a)
      if (lose) {
        lose = false
        throw new TypeError('network')
      }
      return {}
    }
    await queueCommentOp(o, HQ, add('one'), now)
    await replayComments(o, HQ, hq.api)
    expect(await waitingComments(o, HQ, P)).toHaveLength(1)
    await replayComments(o, HQ, hq.api)
    expect(hq.comments.filter((c) => c.text === 'one')).toHaveLength(1)
    expect(await o.list(HQ)).toEqual([])
  })

  test('a resolve that was already applied is not sent again', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([cm({ index: 3, state: 'resolved', hash: 'c2' })])
    await queueCommentOp(o, HQ, { kind: 'comment-resolve', path: P, index: 3, hash: 'c1' }, now)
    await replayComments(o, HQ, hq.api)
    expect(hq.calls).toEqual(['list'])
    expect(await o.list(HQ)).toEqual([])
  })

  test('still unreachable: everything stays queued, in order, for the next try', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([], () => new TypeError('offline'))
    await queueCommentOp(o, HQ, add('one'), now)
    await queueCommentOp(o, HQ, add('two'), now)
    const r = await replayComments(o, HQ, hq.api)
    expect(r).toEqual({ sent: [], refused: 0 })
    expect((await waitingComments(o, HQ, P)).map((w) => (w.op as { text: string }).text)).toEqual(['one', 'two'])
  })

  test('a stale refusal is kept with its sentence and the text, and never sent again', async () => {
    const o = createOutbox(memoryStore())
    const sentence = 'The block you commented on has changed.'
    const hq = fakeHq()
    await queueCommentOp(o, HQ, add('my words'), now)
    await queueCommentOp(o, HQ, add('later'), now)
    const api: CommentApi = { ...hq.api, add: (p, a) => (a.text === 'my words' ? Promise.reject(new HqHttpError(sentence, 422)) : hq.api.add(p, a)) }
    const r = await replayComments(o, HQ, api)
    expect(r.refused).toBe(1)
    // The one behind it still went through.
    expect(hq.comments.map((c) => c.text)).toEqual(['later'])
    expect(await waitingComments(o, HQ, P)).toEqual([])
    const kept = await refusedComments(o, HQ, P)
    expect(kept).toHaveLength(1)
    expect(kept[0].reason).toBe(sentence)
    expect(kept[0].op).toMatchObject({ kind: 'comment-add', text: 'my words' })
    // A later drain never sends it.
    const again = fakeHq()
    await replayComments(o, HQ, again.api)
    expect(again.calls).toEqual([])
    await discardCommentOp(o, HQ, kept[0].clientId)
    expect(await refusedComments(o, HQ, P)).toEqual([])
  })

  test('a 409 on a resolve is kept visible too', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([cm({})], (call) => (call === 'resolve' ? new HqHttpError('That comment changed.', 409) : undefined))
    await queueCommentOp(o, HQ, { kind: 'comment-resolve', path: P, index: 3, hash: 'old', text: 'Check' }, now)
    const r = await replayComments(o, HQ, hq.api)
    expect(r.refused).toBe(1)
    expect((await refusedComments(o, HQ, P))[0]).toMatchObject({ reason: 'That comment changed.', op: { kind: 'comment-resolve' } })
  })

  test('a server error is retried later, not kept as a refusal', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq([], (call) => (call === 'add' ? new HqHttpError('boom', 500) : undefined))
    await queueCommentOp(o, HQ, add('one'), now)
    await replayComments(o, HQ, hq.api)
    expect(await waitingComments(o, HQ, P)).toHaveLength(1)
    expect(await refusedComments(o, HQ, P)).toEqual([])
  })

  test('two replays at once send each operation once', async () => {
    const o = createOutbox(memoryStore())
    const hq = fakeHq()
    await queueCommentOp(o, HQ, add('one'), now)
    await Promise.all([replayComments(o, HQ, hq.api), replayComments(o, HQ, hq.api)])
    expect(hq.calls.filter((c) => c.startsWith('add'))).toEqual(['add:one'])
  })

  test('today is a plain date', () => {
    expect(today(new Date(2026, 10, 5))).toBe('2026-11-05')
  })
})

describe('the list with waiting comments laid over it', () => {
  const blocks = [{ index: 0, hash: 'b1', kind: 'paragraph', src: '\nFirst line\nmore' }]
  const waiting = (op: Parameters<typeof queueCommentOp>[2], id = 'w1') => ({ clientId: id, op })
  test('a waiting add shows at once, after HQ\'s own, marked and anchored', () => {
    const out = mergeQueued([cm({})], [waiting(add('new'))], blocks)
    expect(out).toHaveLength(2)
    expect(out[1]).toMatchObject({ text: 'new', waitingId: 'w1', state: 'open', anchor_preview: 'First line', by: 'Sam' })
    expect(out[1].index).toBeLessThan(0)
  })
  test('a waiting resolve marks its comment and leaves the rest alone', () => {
    const out = mergeQueued([cm({}), cm({ index: 4, hash: 'c2' })], [waiting({ kind: 'comment-resolve', path: P, index: 3, hash: 'c1' })])
    expect(out.map((c) => c.resolving ?? false)).toEqual([true, false])
  })
  test('an add HQ already holds shows once', () => {
    const out = mergeQueued([cm({ text: 'new', at: '2026-11-21' })], [waiting(add('new'))])
    expect(out).toHaveLength(1)
    expect(out[0].waitingId).toBeUndefined()
  })
  test('with nothing cached, the waiting ones alone are listed', () => {
    expect(mergeQueued([], [waiting(add('a'), 'w1'), waiting(add('b'), 'w2')]).map((c) => c.text)).toEqual(['a', 'b'])
  })
})
