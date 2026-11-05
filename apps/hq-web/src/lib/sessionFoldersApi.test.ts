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

const NOW = new Date(2026, 9, 29, 15, 0, 0).getTime()
const secs = (ms: number) => Math.floor(ms / 1000)

test('an approval older than today is a date, never a bare clock time', () => {
  const twelveDays = NOW - 12 * 86_400_000
  const date = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' }).format(twelveDays)
  expect(formatApprovedText(secs(twelveDays), 'claude', NOW)).toBe(`approved ${date} for Claude`)
  expect(formatApprovedText(secs(NOW - 86_400_000), 'codex', NOW)).toBe('approved yesterday for Codex')
  expect(formatApprovedText(secs(twelveDays), 'claude', NOW)).not.toContain(clockTime(secs(twelveDays)))
})

test('revokeConfirmMessage formats confirmation prompt', () => {
  expect(revokeConfirmMessage('/work/repo', 'laptop')).toBe(
    'Stop allowing helpers in /work/repo on laptop?',
  )
})
