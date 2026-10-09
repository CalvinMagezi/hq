import { afterEach, expect, test } from 'bun:test'
import { INTERRUPT_KEY, KEY_NAME_PATTERN, QUICK_KEYS, attachCaveat, attachCommand, canSend, globalSessionsApi, isBlocked, needsAttention, watchingLabel, workbenchApi, type WatchedSession } from './sessionsApi'

test('attachCommand lists a remote host\'s agents over ssh and a local one directly', () => {
  expect(attachCommand({ host: 'local' })).toBe('hq host status')
  expect(attachCommand({ host: 'native' })).toBe('hq host status')
  expect(attachCommand({ host: 'laptop' })).toBe('ssh laptop hq host status')
})

test('attachCaveat warns only for remote hosts, where the name is assumed to be an ssh alias', () => {
  expect(attachCaveat({ host: 'local' })).toBeNull()
  expect(attachCaveat({ host: 'native' })).toBeNull()
  expect(attachCaveat({ host: 'laptop' })).toContain('"laptop" is an ssh alias')
})

test('canSend is false for ended sessions and unreachable hosts', () => {
  expect(canSend({ status: 'running', alive: true, reachable: true })).toBe(true)
  expect(canSend({ status: 'exited', alive: false })).toBe(false)
  expect(canSend({ status: 'running', alive: null, reachable: false })).toBe(false)
})

test('isBlocked needs a running session waiting at a dialog', () => {
  expect(isBlocked({ status: 'running', agent_status: 'blocked' })).toBe(true)
  expect(isBlocked({ status: 'stopped', agent_status: 'blocked' })).toBe(false)
})

const session = (over: Partial<WatchedSession>): WatchedSession => ({
  id: 'hs-1',
  harness: 'claude-code',
  label: 'x',
  host: 'laptop',
  cwd: '/r',
  status: 'running',
  agent_status: 'working',
  last_seen_at: null,
  drive: true,
  mode: 'drive',
  goal: null,
  done_criteria: null,
  drive_blocked_by: [],
  drive_off_reason: null,
  nudges_sent: 0,
  pending_wake: null,
  last_driven_at: null,
  created_at: '2026-09-26 00:00:00',
  task: null,
  ...over,
})

test('a blocked or woken running session needs attention, an exited one does not', () => {
  expect(needsAttention(session({}))).toBe(false)
  expect(needsAttention(session({ agent_status: 'blocked' }))).toBe(true)
  expect(needsAttention(session({ pending_wake: 'finished' }))).toBe(true)
  expect(needsAttention(session({ status: 'exited', pending_wake: 'exited' }))).toBe(false)
})

test('the Watching button names its count and what is waiting', () => {
  expect(watchingLabel([session({})])).toBe('Watching 1 session')
  expect(watchingLabel([session({}), session({ id: 'hs-2', agent_status: 'blocked' })])).toBe(
    'Watching 2 sessions, 1 need attention',
  )
})

test('every key the send box offers is one the server accepts, and interrupt is kept apart from the quick keys', () => {
  for (const key of [...QUICK_KEYS, INTERRUPT_KEY]) expect(KEY_NAME_PATTERN.test(key)).toBe(true)
  expect((QUICK_KEYS as readonly string[]).includes(INTERRUPT_KEY)).toBe(false)
  for (const bad of ['-x', '--help', 'a b', '$(id)', 'k'.repeat(33)]) expect(KEY_NAME_PATTERN.test(bad)).toBe(false)
})

interface Call {
  url: string
  method: string
  body: unknown
}
const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

/** Replaces fetch with a recorder that answers every request with an empty object. */
function recordCalls(): Call[] {
  const calls: Call[] = []
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ? JSON.parse(String(init.body)) : undefined })
    return new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } })
  }) as unknown as typeof fetch
  return calls
}

test('the Workbench reads computers and folders with the right urls', async () => {
  const calls = recordCalls()
  await workbenchApi.hosts()
  await workbenchApi.dirs('native')
  await workbenchApi.dirs('my pc', '/r/HQ/a b')
  expect(calls.map((c) => [c.method, c.url])).toEqual([
    ['GET', '/api/workbench/hosts'],
    ['GET', '/api/workbench/hosts/native/dirs'],
    ['GET', '/api/workbench/hosts/my%20pc/dirs?path=%2Fr%2FHQ%2Fa%20b'],
  ])
})

test('making a folder and starting an agent post the expected bodies', async () => {
  const calls = recordCalls()
  await workbenchApi.createDir('native', { parent: '/r/HQ', name: 'new' })
  await workbenchApi.spawn({ harness: 'claude-code', host: 'native', folder: '/r/HQ/new', prompt: 'hi' })
  expect(calls).toEqual([
    { url: '/api/workbench/hosts/native/dirs', method: 'POST', body: { parent: '/r/HQ', name: 'new' } },
    { url: '/api/harness-sessions', method: 'POST', body: { harness: 'claude-code', host: 'native', folder: '/r/HQ/new', prompt: 'hi' } },
  ])
})

test('stop, resume, rename and archive hit their own endpoints', async () => {
  const calls = recordCalls()
  await globalSessionsApi.stop('hs-1')
  await globalSessionsApi.resume('hs-1')
  await globalSessionsApi.resume('hs-1', 'go on')
  await globalSessionsApi.rename('hs-1', 'Fix login')
  await globalSessionsApi.archive('hs-1', true)
  await globalSessionsApi.archive('hs-1', false)
  expect(calls).toEqual([
    { url: '/api/harness-sessions/hs-1/stop', method: 'POST', body: undefined },
    { url: '/api/harness-sessions/hs-1/resume', method: 'POST', body: {} },
    { url: '/api/harness-sessions/hs-1/resume', method: 'POST', body: { prompt: 'go on' } },
    { url: '/api/harness-sessions/hs-1/rename', method: 'POST', body: { label: 'Fix login' } },
    { url: '/api/harness-sessions/hs-1/archive', method: 'POST', body: { archived: true } },
    { url: '/api/harness-sessions/hs-1/archive', method: 'POST', body: { archived: false } },
  ])
})

test('the list asks for archived agents only when told to', async () => {
  const calls = recordCalls()
  await globalSessionsApi.list()
  await globalSessionsApi.list({ include_archived: true, task_id: 't1' })
  expect(calls.map((c) => c.url)).toEqual(['/api/harness-sessions', '/api/harness-sessions?include_archived=true&task_id=t1'])
})
