import { expect, test } from 'bun:test'
import type { ThreadMessage } from '~/store/threadStore'
import { isMcpLocked } from './mcpLock'

const msg = (role: 'user' | 'assistant', viaMcp?: string): ThreadMessage =>
  ({ messageId: `${role}-${viaMcp ?? 'x'}`, threadId: 't', role, content: 'c', createdAt: 0, viaMcp }) as ThreadMessage

test('an MCP question and its direct reply are locked', () => {
  const list = [msg('user', 'claude-code'), msg('assistant')]
  expect(isMcpLocked(list, 0)).toBe(true)
  expect(isMcpLocked(list, 1)).toBe(true)
})

test('normal messages stay editable', () => {
  const list = [msg('user'), msg('assistant')]
  expect(isMcpLocked(list, 0)).toBe(false)
  expect(isMcpLocked(list, 1)).toBe(false)
})

test('only the reply directly after an MCP question is locked', () => {
  const list = [msg('user', 'mcp'), msg('assistant'), msg('user'), msg('assistant')]
  expect(isMcpLocked(list, 3)).toBe(false)
  expect(isMcpLocked(list, 2)).toBe(false)
  expect(isMcpLocked(list, 9)).toBe(false)
})
