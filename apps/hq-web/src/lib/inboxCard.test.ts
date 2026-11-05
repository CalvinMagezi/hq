import { expect, test } from 'bun:test'
import { answerErrorLine, cardKind, hideNotifyCard, lapsed, notifyCardHidden, NOTIFY_HIDE_MS, REQUEST_LIFETIME_MS } from './inboxCard'
import { clearableUpdateIds, inInboxTab, type NotificationItem } from './notificationsApi'
import { t } from '~/strings'

const item = (o: Partial<NotificationItem> & { extra?: Record<string, string> } = {}): NotificationItem =>
  ({
    id: 'n1',
    kind: 'action_needed',
    category: 'x',
    title: 'T',
    description: '',
    state: 'pending',
    created_at: new Date().toISOString(),
    metadata: { extra: o.extra, ...(o.metadata ?? {}) },
    actions: [],
    ...o,
  }) as NotificationItem

test('a 409 on an answer says the question changed and nothing was done', () => {
  expect(answerErrorLine({ status: 409 })).toBe(t('fail_send_answer_changed'))
  expect(answerErrorLine({ status: 500 })).toBe(t('fail_send_answer_no_reply'))
  expect(answerErrorLine(new TypeError('x'))).toBe(t('fail_send_answer_unreachable'))
})

test('a request older than 24 hours lapses, a younger one does not', () => {
  const now = Date.now()
  const old = item({ created_at: new Date(now - REQUEST_LIFETIME_MS - 1000).toISOString(), metadata: { approval: { tool: 'x' } } as never })
  const fresh = item({ metadata: { approval: { tool: 'x' } } as never })
  expect(lapsed(old, now)).toBe(true)
  expect(lapsed(fresh, now)).toBe(false)
})

test('card kinds: stuck helper, task check and update', () => {
  expect(cardKind(item({ extra: { source_task: 'session-supervisor' } }))).toBe('stuck')
  // A notice to look at (Clear only) is an update, never an approval card.
  expect(cardKind(item({ kind: 'action_needed', actions: ['dismiss'] }))).toBe('update')
  expect(cardKind(item({ kind: 'action_needed', actions: ['approve', 'reject'] }))).toBe('approval')
  expect(cardKind(item({ extra: { source_task: 'task_ready_for_review' } }))).toBe('task_check')
  expect(cardKind(item({ kind: 'value_item' }))).toBe('update')
})

test('clearing updates never touches Needs you items', () => {
  const list = [item({ id: 'a' }), item({ id: 'b', kind: 'value_item' })]
  expect(clearableUpdateIds(list)).toEqual(['b'])
  expect(inInboxTab(list[0], 'pending')).toBe(true)
  expect(inInboxTab(list[1], 'pending')).toBe(false)
})

test('Not now hides the notifications card for 7 days', () => {
  const m = new Map<string, string>()
  const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) }
  expect(notifyCardHidden(1000, store)).toBe(false)
  hideNotifyCard(1000, store)
  expect(notifyCardHidden(1000 + NOTIFY_HIDE_MS - 1, store)).toBe(true)
  expect(notifyCardHidden(1000 + NOTIFY_HIDE_MS + 1, store)).toBe(false)
})

test('Done is grouped by day with Today and Yesterday headings', async () => {
  const { groupByDay } = await import('./inboxCard')
  const now = Date.UTC(2026, 5, 10, 15)
  const at = (h: number) => new Date(now - h * 3600_000).toISOString()
  const g = groupByDay([item({ id: 'a', created_at: at(1) }), item({ id: 'b', created_at: at(2) }), item({ id: 'c', created_at: at(20) })], now, 'UTC')
  expect(g.map((x) => x.heading)).toEqual([t('chat_group_today'), t('time_yesterday')])
  expect(g[0].items).toHaveLength(2)
})

test('an item from a server HQ leads with On your server HQ', async () => {
  const { serverHqLine } = await import('./inboxCard')
  expect(serverHqLine(item({ extra: { server_hq: '1', server_hq_name: 'Office' } }))).toBe('On your server HQ · Office')
  expect(serverHqLine(item())).toBeNull()
})

test('a helper-supervisor item without a dialog is a stuck helper', () => {
  expect(cardKind(item({ extra: { source_task: 'session-supervisor' }, kind: 'info' as never }))).toBe('stuck')
})
