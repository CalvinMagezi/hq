import { describe, expect, test } from 'bun:test'
import { digestOf, LATER_MS, neighbour, preferredItem, readLater, splitHelpers, splitLater, writeLater } from './inboxView'
import type { NotificationItem } from './notificationsApi'

const item = (id: string, at: string, extra: Partial<NotificationItem['metadata']> = {}, state: NotificationItem['state'] = 'pending'): NotificationItem => ({
  id, kind: 'action_needed', category: 'x', title: id, description: '', state, created_at: at, metadata: extra, actions: [],
})
const helper = (id: string) => item(id, '2026-11-17T10:00:00Z', { extra: { source_task: 'session-supervisor' } })

describe('which item opens by itself', () => {
  test('a sign-in first, then the oldest waiting request', () => {
    const list = [item('b', '2026-11-17T12:00:00Z'), item('a', '2026-11-17T09:00:00Z'), item('s', '2026-11-17T13:00:00Z', { sign_in: { device_id: 'd' } })]
    expect(preferredItem(list)?.id).toBe('s')
    expect(preferredItem(list.slice(0, 2))?.id).toBe('a')
  })
  test('nothing when nothing waits', () => {
    expect(preferredItem([])).toBeNull()
    expect(preferredItem([item('x', '2026-11-17T09:00:00Z', {}, 'approved')])).toBeNull()
  })
})

describe('helpers in the list', () => {
  test('their requests fold together and the owner own stay', () => {
    const { mine, helpers } = splitHelpers([helper('h1'), item('m', '2026-11-17T09:00:00Z'), helper('h2')])
    expect(helpers.map((n) => n.id)).toEqual(['h1', 'h2'])
    expect(mine.map((n) => n.id)).toEqual(['m'])
  })
  test('three helper updates in a day become one digest, two stay rows', () => {
    expect(digestOf([helper('1'), helper('2')]).helpers).toEqual([])
    const d = digestOf([helper('1'), helper('2'), helper('3'), item('o', '2026-11-17T09:00:00Z')])
    expect(d.helpers).toHaveLength(3)
    expect(d.rows.map((n) => n.id)).toEqual(['o'])
  })
})

describe('moving with J and K', () => {
  const list = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]
  test('steps along the list and stops at the ends', () => {
    expect(neighbour(list, 'a', 1)).toBe('b')
    expect(neighbour(list, 'c', 1)).toBe('c')
    expect(neighbour(list, 'a', -1)).toBe('a')
  })
  test('with nothing open, J takes the first and K the last', () => {
    expect(neighbour(list, undefined, 1)).toBe('a')
    expect(neighbour(list, undefined, -1)).toBe('c')
    expect(neighbour([], 'a', 1)).toBeNull()
  })
})

describe('waiting for later', () => {
  const store = () => {
    const data: Record<string, string> = {}
    return { getItem: (k: string) => data[k] ?? null, setItem: (k: string, v: string) => void (data[k] = v) }
  }
  test('is kept in this browser, and a damaged value reads as nothing', () => {
    const s = store()
    writeLater({ a: 5 }, s)
    expect(readLater(s)).toEqual({ a: 5 })
    s.setItem('hq-inbox-later', '[1')
    expect(readLater(s)).toEqual({})
    expect(readLater({ getItem: () => { throw new Error('blocked') } })).toEqual({})
  })
  test('an update comes back when its time is up', () => {
    const list = [item('a', '2026-11-17T09:00:00Z'), item('b', '2026-11-17T09:00:00Z')]
    const now = 1_000
    expect(splitLater(list, { a: now + LATER_MS }, now).waiting.map((n) => n.id)).toEqual(['a'])
    expect(splitLater(list, { a: now - 1 }, now).waiting).toEqual([])
  })
})
