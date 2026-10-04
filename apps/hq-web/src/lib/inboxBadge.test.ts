import { expect, test } from 'bun:test'
import { inboxBadge } from './inboxBadge'
import { useHQStore } from '~/store/hqStore'

test('zero, negative and non-finite counts hide the badge', () => {
  expect(inboxBadge(0)).toBeNull()
  expect(inboxBadge(-3)).toBeNull()
  expect(inboxBadge(NaN)).toBeNull()
})

test('positive counts show the number with accessible text', () => {
  expect(inboxBadge(1)).toEqual({ label: '1', aria: '1 unread notification' })
  expect(inboxBadge(7)).toEqual({ label: '7', aria: '7 unread notifications' })
})

test('large counts are capped in the label but exact in the aria text', () => {
  expect(inboxBadge(99)?.label).toBe('99')
  expect(inboxBadge(100)?.label).toBe('99+')
  expect(inboxBadge(1234)).toEqual({ label: '99+', aria: '1234 unread notifications' })
})

test('badge follows the store count the desktop indicator reads', () => {
  const { setNotificationCounts } = useHQStore.getState()
  setNotificationCounts(0, 0)
  expect(inboxBadge(useHQStore.getState().unreadNotificationsCount)).toBeNull()
  setNotificationCounts(4, 1)
  expect(inboxBadge(useHQStore.getState().unreadNotificationsCount)?.label).toBe('4')
  setNotificationCounts(150, 1)
  expect(inboxBadge(useHQStore.getState().unreadNotificationsCount)?.label).toBe('99+')
})
