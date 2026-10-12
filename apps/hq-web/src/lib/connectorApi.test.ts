import { expect, test } from 'bun:test'
import { SCOPE_CHOICES, safeRedirect } from './connectorApi'

test('only Claude callbacks and loopback callbacks are followed', () => {
  expect(safeRedirect('https://claude.ai/api/mcp/auth_callback?code=c&state=s')).toBe(
    'https://claude.ai/api/mcp/auth_callback?code=c&state=s',
  )
  expect(safeRedirect('https://claude.com/api/mcp/auth_callback?error=access_denied')).not.toBeNull()
  expect(safeRedirect('http://localhost:3118/callback?code=c')).not.toBeNull()
  expect(safeRedirect('http://127.0.0.1:55001/callback?code=c')).not.toBeNull()
  for (const bad of [
    'http://claude.ai/api/mcp/auth_callback',
    'https://evil.example/cb',
    'https://claude.ai.evil.example/cb',
    'javascript:alert(1)',
    'https://localhost/callback',
    '/relative',
    '',
  ]) {
    expect(safeRedirect(bad)).toBeNull()
  }
})

test('scopes are offered narrowest first', () => {
  expect(SCOPE_CHOICES.map((s) => s.id)).toEqual(['read_only', 'tasks', 'full'])
})
