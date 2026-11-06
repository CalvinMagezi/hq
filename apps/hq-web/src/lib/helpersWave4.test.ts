import { afterEach, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { LockdownBanner } from '~/components/sessions/EmergencyStop'
import { BlockedCallout } from '~/components/sessions/BlockedCallout'
import { TechnicalDetails } from '~/components/sessions/TechnicalDetails'
import { lockFact, unreachableHosts } from './lockdown'
import { INTERRUPT_ARM_MS, interruptStep, terminalAction } from './terminalKeys'
import { globalSessionsApi, type HarnessSession } from './sessionsApi'
import { autoAnswers } from './autoAnswers'
import { t } from '~/strings'

const key = (k: string, extra: Record<string, unknown> = {}) => ({ key: k, ...extra })

test('Esc in the screen with typing off leaves it and sends nothing; with typing on it is sent', () => {
  expect(terminalAction(key('Escape'), false, false)).toEqual({ do: 'leave' })
  expect(terminalAction(key('Escape'), true, false)).toEqual({ do: 'send', key: 'esc' })
})

test('Ctrl ] always leaves, on any layout, typing on or off', () => {
  expect(terminalAction({ key: '}', code: 'BracketRight', ctrlKey: true }, true, false)).toEqual({ do: 'leave' })
  expect(terminalAction({ key: ']', code: 'BracketRight', ctrlKey: true }, false, false)).toEqual({ do: 'leave' })
})

test('keys go to the helper only while typing is on; other keys are ignored', () => {
  expect(terminalAction(key('y'), false, false)).toEqual({ do: 'none' })
  expect(terminalAction(key('y'), true, false)).toEqual({ do: 'send', key: 'y' })
  expect(terminalAction(key('ArrowUp'), true, false)).toEqual({ do: 'send', key: 'up' })
  expect(terminalAction(key('q'), true, false)).toEqual({ do: 'none' })
})

test('Ctrl C interrupts, but copies when text is selected, and needs a second press in 4 s', () => {
  expect(terminalAction(key('c', { ctrlKey: true }), true, false)).toEqual({ do: 'interrupt' })
  expect(terminalAction(key('c', { ctrlKey: true }), true, true)).toEqual({ do: 'none' })
  const first = interruptStep(null, 1_000)
  expect(first).toEqual({ send: false, armedAt: 1_000 })
  expect(interruptStep(first.armedAt, 1_000 + INTERRUPT_ARM_MS)).toEqual({ send: true, armedAt: null })
  expect(interruptStep(first.armedAt, 1_000 + INTERRUPT_ARM_MS + 1).send).toBe(false)
})

const host = (over: Record<string, unknown>) => ({ host: 'native', reachable: true, ...over }) as never

test('the lock banner says the words, names who stopped it, and has no button to lift it', () => {
  const html = renderToStaticMarkup(
    createElement(LockdownBanner, { hosts: [host({ lockdown: { reason: 'x', source: 'operator', at: 1_762_000_000 } })] }),
  )
  expect(html).toContain(t('helpers_locked_banner'))
  expect(html).not.toContain('<button')
  expect(html).not.toContain('Allow helpers again')
  expect(html).not.toContain('hq host unlock on it to start')
})

test('a lock set by the safety breaker says so, and a computer that is not locked shows no banner', () => {
  expect(lockFact('native', { reason: '', source: 'breaker', at: 1_762_000_000 })).toContain('too many started at once')
  expect(renderToStaticMarkup(createElement(LockdownBanner, { hosts: [host({})] }))).toBe('')
})

test('Stop every helper names the computers that could not be reached', () => {
  const result = { all_locked: false, hosts: [{ host: 'a', locked: true }, { host: 'b', locked: false }] }
  expect(unreachableHosts(result)).toEqual(['b'])
  expect(t('stop_all_unreachable')).toBe("That computer can't be reached, so its helpers were not stopped.")
})

const blocked = {
  id: 'hs-1', harness: 'claude', label: 'Gradle', host: 'native', cwd: '/x', status: 'running', agent_status: 'blocked',
  pending_seq: 3,
  pending: { kind: 'command', command: './gradlew assembleDebug --offline', question: null, truncated: false, options: [{ id: 1, label: 'Yes' }] },
} as unknown as HarnessSession

test('the answer card sits in the page: it is not sticky, and the question comes before the buttons', () => {
  const html = renderToStaticMarkup(createElement(BlockedCallout, { session: blocked, screenText: '', onSent() {} }))
  expect(html).not.toContain('sticky')
  expect(html.indexOf('./gradlew assembleDebug --offline')).toBeGreaterThan(-1)
  expect(html.indexOf('./gradlew assembleDebug')).toBeLessThan(html.indexOf('Decline'))
})

test('Technical details is closed by default, so no screen is read or streamed', () => {
  const html = renderToStaticMarkup(createElement(TechnicalDetails, { session: blocked, refreshKey: 0, pollMs: 5000, onText() {}, onSent() {} }))
  expect(html).toContain('aria-expanded="false"')
  expect(html).toContain(t('helpers_technical_hint'))
  expect(html).not.toContain('role="log"')
  expect(html).not.toContain(t('helpers_typing_label'))
})

test('the details panel docks at 1400, not 1536', () => {
  const source = readFileSync(join(import.meta.dir, '..', 'routes', 'helpers.tsx'), 'utf8')
  expect(source).toContain('min-[1400px]:flex')
  expect(source).not.toContain('2xl:flex')
})

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

test('what HQ said yes to is read from the answers route, never from the session payload', async () => {
  const urls: string[] = []
  globalThis.fetch = (async (url: string) => {
    urls.push(url)
    const answers = [{ by: 'a Drive turn', at: '2026-11-05T11:12:00Z', kind: 'command', choice: { option: 1 }, registered: true }]
    return new Response(JSON.stringify({ answers }), { status: 200, headers: { 'Content-Type': 'application/json' } })
  }) as unknown as typeof fetch
  const list = await globalSessionsApi.answers('hs 1')
  expect(urls).toEqual(['/api/harness-sessions/hs%201/answers'])
  expect(autoAnswers(list).map((a) => a.by)).toEqual(['a Drive turn'])
})

test('a task page starts a helper on that task, and the start sends it (WR17)', () => {
  const src = (p: string) => readFileSync(join(import.meta.dir, p), 'utf8')
  const sessions = src('../components/tasks/TaskSessions.tsx')
  expect(sessions).toContain("t('helpers_start_on_task')")
  expect(sessions).toContain('task={{ id: taskId, title: taskTitle }}')
  expect(src('../components/sessions/NewAgentDialog.tsx')).toContain('task_id: task.id')
})
