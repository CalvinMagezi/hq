import { expect, test } from 'bun:test'
import { clockTime } from './connectorApi'
import {
  SESSION_FOLDERS_QUERY_KEY,
  formatApprovedText,
  revokeConfirmMessage,
} from './sessionFoldersApi'

test('query key is session-folders', () => {
  expect(SESSION_FOLDERS_QUERY_KEY).toEqual(['session-folders'])
})

test('formatApprovedText formats timestamp and requester', () => {
  const ts = 1700000000
  const expectedTime = clockTime(ts)
  expect(formatApprovedText(ts, 'claude')).toBe(`approved ${expectedTime} for claude`)
})

test('revokeConfirmMessage formats confirmation prompt', () => {
  expect(revokeConfirmMessage('/work/repo', 'laptop')).toBe(
    'Stop allowing sessions in /work/repo on laptop?',
  )
})
