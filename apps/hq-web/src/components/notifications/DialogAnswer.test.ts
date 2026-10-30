import { describe, expect, test } from 'bun:test'
import type { NotificationItem } from '~/lib/notificationsApi'
import { openDialog } from './DialogAnswer'

const dialog = {
  session_id: 'hs-1',
  seq: 7,
  kind: 'other',
  question: 'Which colour should the test use?',
  command: null,
  options: [
    { id: 1, label: 'Red' },
    { id: 2, label: 'Blue' },
  ],
  approve: null,
  answered: false,
}

const notice = (over: Partial<NotificationItem>, d: typeof dialog | null = dialog): NotificationItem => ({
  id: 'val_1',
  kind: 'action_needed',
  category: 'value',
  title: "Harness session 'claude' is waiting for you",
  description: '',
  state: 'pending',
  created_at: '2026-10-30T00:00:00Z',
  metadata: d ? { dialog: d } : {},
  actions: ['dismiss'],
  ...over,
})

describe('openDialog', () => {
  test('a pending notice of an unanswered dialog is answered through the dialog', () => {
    expect(openDialog(notice({}))?.question).toBe('Which colour should the test use?')
  })
  test('an answered, resolved or plain notice keeps its own buttons', () => {
    expect(openDialog(notice({}, { ...dialog, answered: true }))).toBeNull()
    expect(openDialog(notice({ state: 'approved' }))).toBeNull()
    expect(openDialog(notice({}, null))).toBeNull()
  })
})
