import { expect, test } from 'bun:test'
import type { HarnessSession } from './sessionsApi'
import {
  BUSY_POLL_MS,
  READY_POLL_MS,
  agentName,
  archivable,
  computerName,
  computerUnavailableReason,
  crumbs,
  explorerLine,
  folderName,
  groupSessions,
  needsYouCount,
  screenPollMs,
  sessionTitle,
  statusInfo,
  tailLines,
} from './workbench'

const session = (over: Partial<HarnessSession>): HarnessSession => ({
  id: 'hs-1',
  harness: 'claude-code',
  agent_name: 'claude-code',
  label: '',
  host: 'native',
  cwd: '/Users/me/HQ/site',
  status: 'running',
  agent_status: 'working',
  last_seen_at: null,
  drive: false,
  mode: 'observe',
  goal: null,
  done_criteria: null,
  drive_blocked_by: [],
  drive_off_reason: null,
  nudges_sent: 0,
  pending_wake: null,
  last_driven_at: null,
  created_at: '2026-10-01 10:00:00',
  task: null,
  owner_thread: null,
  archived: false,
  ...over,
})

test('known agents get friendly names and unknown ones are shown as they are', () => {
  expect(agentName('claude-code')).toBe('Claude Code')
  expect(agentName('github-copilot')).toBe('GitHub Copilot')
  expect(agentName('opencode')).toBe('OpenCode')
  expect(agentName('mystery-agent')).toBe('mystery-agent')
})

test('this computer covers both native and local, other names pass through', () => {
  expect(computerName('native')).toBe('This computer')
  expect(computerName('local')).toBe('This computer')
  expect(computerName('office-pc')).toBe('office-pc')
})

test('the project folder is the last path segment on either kind of path', () => {
  expect(folderName('/Users/me/HQ/site')).toBe('site')
  expect(folderName('/Users/me/HQ/site/')).toBe('site')
  expect(folderName('C:\\Users\\me\\HQ\\shop')).toBe('shop')
  expect(folderName('')).toBe('')
})

test('the title is the label, else the agent in its folder', () => {
  expect(sessionTitle(session({ label: 'Fix login' }))).toBe('Fix login')
  expect(sessionTitle(session({ label: '  ' }))).toBe('Claude Code in site')
  expect(sessionTitle(session({ harness: 'codex', cwd: '' }))).toBe('Codex')
})

test('every state has a plain status word', () => {
  const word = (over: Partial<HarnessSession>) => statusInfo(session(over)).word
  expect(word({ agent_status: 'working' })).toBe('Working')
  expect(word({ agent_status: 'blocked' })).toBe('Waiting for you')
  expect(word({ agent_status: 'idle' })).toBe('Ready')
  expect(word({ agent_status: 'done' })).toBe('Finished')
  expect(word({ status: 'stopped', agent_status: null })).toBe('Stopped')
  expect(word({ status: 'exited', agent_status: null })).toBe('Finished')
  expect(word({ status: 'orphaned', agent_status: null })).toBe('Lost contact')
  expect(word({ reachable: false })).toBe('Computer offline')
  expect(statusInfo(session({ reachable: false })).warn).toBe(true)
})

test('the needs-you group puts blocked agents first, then the newest', () => {
  const waiting = session({ id: 'wake', pending_wake: 'finished', last_seen_at: '2026-10-02 10:00:00' })
  const blocked = session({ id: 'blocked', agent_status: 'blocked', last_seen_at: '2026-10-01 09:00:00' })
  const working = session({ id: 'busy' })
  const groups = groupSessions([waiting, working, blocked], false)
  expect(groups.needsYou.map((s) => s.id)).toEqual(['blocked', 'wake'])
  expect(groups.working.map((s) => s.id)).toEqual(['busy'])
  expect(needsYouCount([waiting, working, blocked])).toBe(2)
})

test('an agent on an offline computer is not counted as needing you', () => {
  const offline = session({ agent_status: 'blocked', reachable: false })
  expect(groupSessions([offline], false).needsYou).toEqual([])
  expect(groupSessions([offline], false).working).toHaveLength(1)
  expect(needsYouCount([offline])).toBe(0)
})

test('working agents are ordered by project folder', () => {
  const a = session({ id: 'a', cwd: '/r/zebra' })
  const b = session({ id: 'b', cwd: '/r/apple' })
  expect(groupSessions([a, b], false).working.map((s) => s.id)).toEqual(['b', 'a'])
})

test('past agents hide archived ones unless asked, and archive-all skips archived', () => {
  const stopped = session({ id: 'stopped', status: 'stopped', agent_status: null })
  const old = session({ id: 'old', status: 'exited', agent_status: null, archived: true })
  expect(groupSessions([stopped, old], false).past.map((s) => s.id)).toEqual(['stopped'])
  expect(groupSessions([stopped, old], true).past.map((s) => s.id).sort()).toEqual(['old', 'stopped'])
  expect(archivable([stopped, old, session({})]).map((s) => s.id)).toEqual(['stopped'])
})

test('the terminal is polled fast while working or blocked and slower when ready or ended', () => {
  expect(screenPollMs({ status: 'running', agent_status: 'working' })).toBe(BUSY_POLL_MS)
  expect(screenPollMs({ status: 'running', agent_status: 'blocked' })).toBe(BUSY_POLL_MS)
  expect(screenPollMs({ status: 'running', agent_status: 'idle' })).toBe(READY_POLL_MS)
  expect(screenPollMs({ status: 'stopped', agent_status: 'working' })).toBe(READY_POLL_MS)
  expect(BUSY_POLL_MS).toBe(1500)
  expect(READY_POLL_MS).toBe(4000)
})

test('tailLines keeps the newest lines', () => {
  expect(tailLines('a\nb\nc\nd\n\n', 2)).toBe('c\nd')
})

test('a computer that is offline or not updated cannot start agents', () => {
  expect(computerUnavailableReason({ host: 'pc', reachable: false })).toBe('pc is offline.')
  expect(computerUnavailableReason({ host: 'pc', reachable: true })).toContain('Update HQ')
  const workspace = { root: '/r', os: 'linux', wsl: false, explorer_path: '' }
  expect(computerUnavailableReason({ host: 'pc', reachable: true, workspace })).toBeNull()
})

test('the breadcrumb runs from the HQ folder down', () => {
  expect(crumbs('', '/r/HQ')).toEqual([{ label: 'HQ folder', path: '' }])
  expect(crumbs('/r/HQ/a/b', '/r/HQ')).toEqual([
    { label: 'HQ folder', path: '' },
    { label: 'a', path: '/r/HQ/a' },
    { label: 'b', path: '/r/HQ/a/b' },
  ])
  expect(crumbs('C:\\HQ\\a', 'C:\\HQ')[1]).toEqual({ label: 'a', path: 'C:\\HQ\\a' })
})

test('the Explorer line appears only under WSL and follows the chosen subfolder', () => {
  const wsl = { root: '/home/me/HQ', os: 'linux', wsl: true, explorer_path: '\\\\wsl$\\Ubuntu\\home\\me\\HQ' }
  expect(explorerLine({ ...wsl, wsl: false }, '')).toBeNull()
  expect(explorerLine(wsl, '')).toBe('Open in Windows Explorer: \\\\wsl$\\Ubuntu\\home\\me\\HQ')
  expect(explorerLine(wsl, '/home/me/HQ/site')).toBe('Open in Windows Explorer: \\\\wsl$\\Ubuntu\\home\\me\\HQ\\site')
})
