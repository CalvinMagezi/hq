import { describe, expect, test } from 'bun:test'
import { clearedDialogs, inBatches, pileOf, staleRequests } from './catchUp'
import type { NotificationItem } from './notificationsApi'

const NOW = Date.parse('2026-11-17T12:00:00Z')
const item = (id: string, over: Partial<NotificationItem> = {}, session = 's1', answered = false): NotificationItem => ({
  id, kind: 'action_needed', category: 'system', title: id, description: '', state: 'pending', created_at: '2026-11-17T11:30:00Z', actions: ['dismiss'],
  metadata: { dialog: { session_id: session, seq: 1, kind: 'q', question: null, command: null, options: [], approve: null, answered } },
  ...over,
})

describe('which waiting requests can be cleared together', () => {
  const sessions = [{ id: 'live', status: 'running' }, { id: 'done', status: 'exited' }]
  test('a helper that is no longer running, or one gone from the list', () => {
    const list = [item('a', {}, 'live'), item('b', {}, 'done'), item('c', {}, 'unknown')]
    expect(staleRequests(list, sessions, NOW).map((n) => n.id)).toEqual(['b', 'c'])
  })
  test('a request that has waited past a day, even from a helper still running', () => {
    const old = item('o', { created_at: '2026-11-15T09:00:00Z' }, 'live')
    expect(staleRequests([old], sessions, NOW).map((n) => n.id)).toEqual(['o'])
  })
  test('never a sign-in, an approval or something already answered', () => {
    const signIn = item('s', { metadata: { sign_in: { device_id: 'd' } } })
    const approval = item('p', { metadata: { approval: { tool: 'x', args: null, loses: false, verb: null } } })
    expect(staleRequests([signIn, approval, item('x', {}, 'done', true), item('u', { kind: 'value_item' }, 'done')], sessions, NOW)).toEqual([])
  })
})

describe('the pile card', () => {
  const waiting = Array.from({ length: 8 }, (_, i) => item(`w${i}`))
  test('shows past five waiting, with how many are old and how many are left', () => {
    expect(pileOf(waiting, waiting.slice(0, 6))).toEqual({ waiting: 8, old: 6, rest: 2 })
  })
  test('does not show for five or fewer, or when nothing is safe to clear', () => {
    expect(pileOf(waiting.slice(0, 5), waiting.slice(0, 3))).toBeNull()
    expect(pileOf(waiting, [])).toBeNull()
  })
})

describe('Done lists what was cleared', () => {
  test('only agent dialog notices that nobody answered', () => {
    const gone = item('g', { state: 'dismissed' })
    const answered = item('a', { state: 'dismissed' }, 's1', true)
    const decision = item('d', { state: 'dismissed', metadata: {} })
    expect(clearedDialogs([gone, answered, decision]).map((n) => n.id)).toEqual(['g'])
  })
})

describe('clearing in batches', () => {
  test('goes at most `limit` at once, keeps the ones that worked and goes on past a failure', async () => {
    let live = 0
    let peak = 0
    const out = await inBatches([1, 2, 3, 4, 5, 6, 7], 3, async (n) => {
      live += 1
      peak = Math.max(peak, live)
      await new Promise((r) => setTimeout(r, 2))
      live -= 1
      if (n === 4) throw new Error('no')
    })
    expect(out).toEqual([1, 2, 3, 5, 6, 7])
    expect(peak).toBeLessThanOrEqual(3)
  })
})
