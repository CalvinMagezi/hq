import { expect, test } from 'bun:test'
import { CHANGE_EVENTS, sharedWithSubscribers } from './WebSocketContext'

test('rule 5: every change event reaches subscribers, and so do the chat events the outbox waits on', () => {
  for (const type of ['inbox:change', 'notes:change', 'helpers:change', 'budget:change', 'settings:change', 'task:sync', 'sessions:sync']) {
    expect(CHANGE_EVENTS.has(type), type).toBe(true)
    expect(sharedWithSubscribers(type), type).toBe(true)
  }
  expect(sharedWithSubscribers('turn_end')).toBe(true)
  expect(sharedWithSubscribers('text_delta')).toBe(false)
})

test('the retired green banner words are gone from the shell', async () => {
  const { readFileSync } = await import('node:fs')
  const src = readFileSync(new URL('../routes/__root.tsx', import.meta.url), 'utf8')
  expect(src).not.toContain('A new vault app version')
  expect(src).not.toContain('Update now')
  expect(src).not.toContain('accent-green')
})
