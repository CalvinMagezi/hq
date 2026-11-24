import { describe, expect, test } from 'bun:test'
import { createOutbox, type OutboxEntry, type OutboxStore } from './outbox'
import { dropWaiting, queueSuiteEdit, replaySuiteEdits, suiteConflicts, waitingCount } from './suiteOutbox'
import type { SaveResult } from './vaultApi'

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
const offline = (e: unknown) => e instanceof TypeError

describe('edits made with no connection', () => {
  test('later edits to the same note replace the text but keep the first hash', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'one' })
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    await queueSuiteEdit(o, HQ, { path: 'N/b.md', base: 'h9', text: 'other' })
    const list = await o.list(HQ)
    expect(list).toHaveLength(2)
    expect(list[0].payload).toEqual({ path: 'N/a.md', base: 'h1', text: 'two' })
    expect(await waitingCount(o, HQ, 'N/a.md')).toBe(1)
    expect(await waitingCount(o, HQ)).toBe(2)
  })

  test('back online each edit is sent against its hash, and the outbox empties', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    const calls: [string, string, string | undefined][] = []
    const r = await replaySuiteEdits(o, HQ, async (p, t, b): Promise<SaveResult> => (calls.push([p, t, b]), { saved: true, hash: 'h2' }), offline)
    expect(calls).toEqual([['N/a.md', 'two', 'h1']])
    expect(r).toEqual({ sent: ['N/a.md'], conflicts: [] })
    expect(await o.list(HQ)).toEqual([])
  })

  test('a note that moved on is not overwritten: the edit is kept as a conflict with its text', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'mine' })
    const r = await replaySuiteEdits(o, HQ, async () => ({ saved: false, changed: true, theirsHash: 'h7' }), offline)
    expect(r).toEqual({ sent: [], conflicts: ['N/a.md'] })
    expect(await waitingCount(o, HQ)).toBe(0)
    const kept = await suiteConflicts(o, HQ)
    expect(kept).toHaveLength(1)
    expect(kept[0]).toMatchObject({ path: 'N/a.md', text: 'mine' })
    // A kept conflict is never sent by a later drain.
    let sent = 0
    await replaySuiteEdits(o, HQ, async () => (sent++, { saved: true }), offline)
    expect(sent).toBe(0)
  })

  test('still unreachable: the edit stays queued for the next try', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    const r = await replaySuiteEdits(o, HQ, async () => Promise.reject(new TypeError('Failed to fetch')), offline)
    expect(r).toEqual({ sent: [], conflicts: [] })
    expect(await waitingCount(o, HQ)).toBe(1)
    expect((await o.list(HQ))[0].attempts).toBe(1)
  })

  test('a refusal that is not an outage does not retry forever', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    await replaySuiteEdits(o, HQ, async () => Promise.reject(new Error('403')), offline)
    expect(await waitingCount(o, HQ)).toBe(0)
  })

  test('a save that reached HQ drops the waiting copies of that note only', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    await queueSuiteEdit(o, HQ, { path: 'N/b.md', base: 'h1', text: 'two' })
    await dropWaiting(o, HQ, 'N/a.md')
    expect((await o.list(HQ)).map((e) => (e.payload as { path: string }).path)).toEqual(['N/b.md'])
  })

  test('another HQ never sees these edits', async () => {
    const o = createOutbox(memoryStore())
    await queueSuiteEdit(o, HQ, { path: 'N/a.md', base: 'h1', text: 'two' })
    expect(await waitingCount(o, 'hq-b')).toBe(0)
  })
})
