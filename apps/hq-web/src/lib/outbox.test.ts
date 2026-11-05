import { describe, expect, test } from 'bun:test'
import { t } from '~/strings'
import {
  MESSAGE_MAX_AGE_MS,
  OUTBOX_LIMIT,
  createOutbox,
  lineFor,
  newClientId,
  outboxEnabled,
  type OutboxEntry,
  type OutboxStore,
} from './outbox'

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

function entry(n: number, over: Partial<OutboxEntry> = {}): OutboxEntry {
  return { client_id: `c${n}`, kind: 'message', hq_id: 'hq-a', payload: n, created_at: 1000 + n, attempts: 0, ...over }
}

describe('outbox', () => {
  test('a message added while the drain is sending is never lost', async () => {
    const o = createOutbox(memoryStore(), () => 2000)
    await o.add(entry(1))
    await o.drain('hq-a', async () => {
      await o.add(entry(9))
      return 'sent'
    })
    expect((await o.list('hq-a')).map((e) => e.client_id)).toEqual(['c9'])
  })

  test('adds at the same moment both land, and the 101st is still refused', async () => {
    const o = createOutbox(memoryStore(), () => 2000)
    const results = await Promise.all(Array.from({ length: OUTBOX_LIMIT + 1 }, (_, i) => o.add(entry(i))))
    expect(results.filter((r) => r.ok).length).toBe(OUTBOX_LIMIT)
    expect((await o.list('hq-a')).length).toBe(OUTBOX_LIMIT)
  })

  test('drains oldest first', async () => {
    const o = createOutbox(memoryStore(), () => 2000)
    await o.add(entry(2))
    await o.add(entry(1))
    const seen: string[] = []
    await o.drain('hq-a', async (e) => (seen.push(e.client_id), 'sent'))
    expect(seen).toEqual(['c1', 'c2'])
    expect(await o.list('hq-a')).toEqual([])
  })

  test('refuses the 101st with the exact sentence', async () => {
    const o = createOutbox(memoryStore())
    for (let i = 0; i < OUTBOX_LIMIT; i++) expect((await o.add(entry(i))).ok).toBe(true)
    const r = await o.add(entry(999))
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.message).toBe("Couldn't add this. 100 things are already waiting to send.")
    expect(t('fail_add_outbox_full')).toBe("Couldn't add this. 100 things are already waiting to send.")
    expect((await o.list('hq-a')).length).toBe(OUTBOX_LIMIT)
  })

  test('the 24 h rule marks messages only', async () => {
    const now = 10 * MESSAGE_MAX_AGE_MS
    const o = createOutbox(memoryStore(), () => now)
    await o.add(entry(1, { created_at: 0 }))
    await o.add(entry(2, { kind: 'note', created_at: 0 }))
    const seen: string[] = []
    await o.drain('hq-a', async (e) => (seen.push(e.client_id), 'sent'))
    expect(seen).toEqual(['c2'])
    const left = await o.list('hq-a')
    expect(left.map((e) => [e.client_id, e.waited])).toEqual([['c1', true]])
    await o.sendNow('hq-a', 'c1')
    await o.drain('hq-a', async (e) => (seen.push(e.client_id), 'sent'))
    expect(seen).toEqual(['c2', 'c1'])
  })

  test('keeps one client id across retries and stops at a retry with progress', async () => {
    const o = createOutbox(memoryStore(), () => 2000)
    const id = newClientId()
    await o.add(entry(1, { client_id: id }))
    await o.add(entry(2))
    await o.add(entry(3))
    const progress: Array<[number, number]> = []
    const seen: string[] = []
    await o.drain(
      'hq-a',
      async (e) => (seen.push(e.client_id), e.client_id === 'c2' ? 'retry' : 'sent'),
      (n, m) => progress.push([n, m]),
    )
    expect(seen).toEqual([id, 'c2'])
    expect(progress).toEqual([
      [1, 3],
      [2, 3],
    ])
    const left = await o.list('hq-a')
    expect(left.map((e) => e.client_id)).toEqual(['c2', 'c3'])
    expect(left[0].attempts).toBe(1)
    await o.drain('hq-a', async () => 'sent')
    expect(await o.list('hq-a')).toEqual([])
  })

  test('two HQ ids never see each other', async () => {
    const o = createOutbox(memoryStore())
    await o.add(entry(1))
    await o.add(entry(2, { hq_id: 'hq-b' }))
    expect((await o.list('hq-a')).map((e) => e.client_id)).toEqual(['c1'])
    expect((await o.list('hq-b')).map((e) => e.client_id)).toEqual(['c2'])
    await o.clear('hq-a')
    expect(await o.list('hq-a')).toEqual([])
    expect((await o.list('hq-b')).length).toBe(1)
    await o.remove('hq-b', 'c2')
    expect(await o.list('hq-b')).toEqual([])
  })

  test('lines and shared computer', () => {
    expect(lineFor('offline', false)).toBe("Saved in this browser. It will try again when you're back online.")
    expect(lineFor('next', true)).toBe('Saved in HQ Hub. It is next.')
    expect(outboxEnabled(true)).toBe(false)
    expect(outboxEnabled(false)).toBe(true)
    expect(newClientId().length).toBeGreaterThanOrEqual(32)
  })
})
