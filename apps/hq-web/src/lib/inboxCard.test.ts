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
  expect(cardKind(item({ extra: { source_task: 'helper_stuck' } }))).toBe('stuck')
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
