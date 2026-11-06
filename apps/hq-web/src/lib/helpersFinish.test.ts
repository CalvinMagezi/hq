import { expect, test } from 'bun:test'
import { RouterProvider, createMemoryHistory, createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { BlockedCallout } from '~/components/sessions/BlockedCallout'
import { runningByHost } from '~/components/sessions/EmergencyStop'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionRow } from '~/components/sessions/SessionRow'
import { activityWord, canAllowOnce, chipCounts, finishedToArchive, groupHelpers, spanWord, taskBanner } from './helperView'
import { ownCardId, ownState, ownTitle, ownWords } from './edgeWorker'
import type { HarnessSession, PendingDialog } from './sessionsApi'
import { t, tn } from '~/strings'

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const root = createRootRoute({ component: node })
  const detail = createRoute({ getParentRoute: () => root, path: '/helpers/$id', component: () => null })
  const router = createRouter({ routeTree: root.addChildren([detail]), history: createMemoryHistory({ initialEntries: ['/'] }) })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const src = (...p: string[]) => readFileSync(join(import.meta.dir, '..', ...p), 'utf8')

const make = (over: Record<string, unknown>) =>
  ({
    id: 'hs-1', harness: 'claude', label: 'Gradle', host: 'native', cwd: '/work/app', status: 'running', agent_status: 'working',
    created_at: '2025-01-01 10:00:00', last_seen_at: '2025-01-01 10:14:00', archived: false, task: null, owner_thread: null,
    pending: null, pending_seq: null, ...over,
  }) as unknown as HarnessSession

const dialog = (over: Partial<PendingDialog> = {}): PendingDialog => ({
  kind: 'command', question: null, command: './gradlew build', truncated: false,
  options: [{ id: 1, label: 'Yes', remember: false, typed: false, selected: true }, { id: 2, label: "Yes, and don't ask again", remember: true, typed: false, selected: false }, { id: 3, label: 'No, tell it what to do', remember: false, typed: true, selected: false }],
  ...over,
})
const fourteenAgo = new Date(Date.now() - 14 * 60_000).toISOString().slice(0, 19).replace('T', ' ')
const needs = make({ agent_status: 'blocked', pending: dialog(), pending_seq: 4, created_at: fourteenAgo })

test('Allow once shows only for a plain yes that is not remembered or typed', () => {
  expect(canAllowOnce(dialog())).toBe(true)
  expect(canAllowOnce(dialog({ options: [{ id: 2, label: 'Yes', remember: true, typed: false, selected: true }] }))).toBe(false)
  expect(canAllowOnce(dialog({ options: [{ id: 3, label: 'Yes', remember: false, typed: true, selected: true }] }))).toBe(false)
  expect(canAllowOnce(dialog({ kind: 'trust' }))).toBe(false)
  expect(canAllowOnce(dialog({ options: [] }))).toBe(false)
})

test('the list groups by state, with lost contact apart and archived hidden until asked for', () => {
  const list = [
    make({ id: 'a', status: 'exited' }),
    make({ id: 'b', status: 'running', alive: false }),
    needs,
    make({ id: 'c', host: 'far', reachable: false }),
    make({ id: 'd', status: 'stopped', archived: true }),
    make({ id: 'e', agent_status: 'idle' }),
  ]
  const names = (show: boolean) => groupHelpers(list, 'all', show).map((g) => g.group)
  expect(names(false)).toEqual(['needs', 'ready', 'offline', 'lost', 'past'])
  expect(names(true).at(-1)).toBe('archived')
  expect(groupHelpers(list, 'finished', false).flatMap((g) => g.items.map((s) => s.id))).toEqual(['a'])
  expect(chipCounts(list)).toEqual({ all: 5, needs: 1, working: 0, finished: 1 })
})

test('archive acts on finished and stopped helpers only, and says how many in plural words', () => {
  const list = [make({ id: 'a', status: 'exited' }), make({ id: 'b' }), make({ id: 'c', status: 'stopped', archived: true })]
  expect(finishedToArchive(list).map((s) => s.id)).toEqual(['a'])
  expect(tn('helpers_archived_n', 3)).toBe('3 helpers archived')
})

test('a card is a link to its page and answers in place with Decline before Allow once', async () => {
  const html = await inRouter(() => createElement(SessionRow, { session: needs, selected: false, onAnswered() {} }))
  expect(html).toContain('<a ')
  expect(html).toContain('/helpers/hs-1')
  expect(html.indexOf(t('notice_action_decline'))).toBeLessThan(html.indexOf(t('notice_action_allow_once')))
  expect(html).toContain('Wants to run a command')
  expect(html).toContain('14 min')
  const plain = await inRouter(() => createElement(SessionRow, { session: make({}), selected: false, onAnswered() {} }))
  expect(plain).not.toContain(t('notice_action_allow_once'))
})

test('an archived card offers Restore beside its link', async () => {
  const html = await inRouter(() => createElement(SessionRow, { session: make({ status: 'stopped', archived: true }), selected: false, onAnswered() {}, onRestore() {} }))
  expect(html).toContain(t('helpers_restore'))
})

test('card lines use the phone words, and the span reads min, h and d', () => {
  expect(activityWord(make({ host: 'far', reachable: false }))).toBe('HQ cannot reach far right now')
  expect(activityWord(make({ alive: false }))).toBe('HQ lost contact with this helper')
  expect(activityWord(make({ agent_status: 'idle' }))).toBe('Waiting for your next instruction')
  const at = (mins: number) => spanWord(make({ created_at: '2025-01-01 10:00:00' }), Date.parse('2025-01-01T10:00:00Z') + mins * 60_000)
  expect([at(14), at(180), at(60 * 24 * 4)]).toEqual(['14 min', '3 h', '4 d'])
})

test('the answer card names the helper, quotes the command, explains both answers and keeps other answers apart', () => {
  const html = renderToStaticMarkup(createElement(BlockedCallout, { session: needs, screenText: 'x', onSent() {} })).replaceAll('&#x27;', "'")
  expect(html).toContain('Gradle wants to run a command')
  expect(html).toContain('./gradlew build')
  expect(html).toContain(t('approval_in_words'))
  expect(html).toContain(t('helpers_if_allow', { helper: 'Gradle' }))
  expect(html).toContain(t('helpers_if_decline', { helper: 'Gradle' }))
  expect(html).toContain(t('helpers_allow_note'))
  expect(html).toContain(t('approval_other_answers'))
  expect(html).toContain(t('helpers_other_answers_hint'))
  expect(html).toContain(t('helpers_tell_instead', { helper: 'Gradle' }))
  expect(html).not.toContain('sticky')
})

test('a stopped helper opens with a Resume panel, an optional next instruction and Archive', () => {
  const html = renderToStaticMarkup(createElement(SessionDetail, { session: make({ status: 'stopped' }), onBack() {}, onChanged() {} }))
  expect(html).toContain(t('helpers_resume_next'))
  expect(html).toContain(t('helpers_resume_act'))
  expect(html).toContain(t('helpers_archive_act'))
  expect(html).toContain('Gradle stopped')
})

test('a working helper shows Right now with the two quick replies in the phone words', () => {
  const html = renderToStaticMarkup(createElement(SessionDetail, { session: make({}), onBack() {}, onChanged() {} }))
  expect(html).toContain(t('helpers_right_now'))
  expect(html).toContain(t('helpers_quick_keep_going'))
  expect(html).toContain(t('helpers_quick_explain'))
  expect(t('helpers_quick_keep_going_text')).toBe('Please keep going.')
  expect(src('components', 'sessions', 'SessionDetail.tsx')).toContain('globalSessionsApi')
  expect(src('components', 'sessions', 'SessionDetail.tsx')).toContain('.answers(')
})

test('HQ own work follows the phone: pending is working, answered is finished, anything else stopped', () => {
  const run = (status: 'pending' | 'answered' | 'failed') => ({ ask_id: 'k1', status, task: { display_id: 'T-4', title: 'Fix login', status: 'x' } }) as never
  expect([ownState(run('pending')), ownState(run('answered')), ownState(run('failed'))]).toEqual(['working', 'finished', 'stopped'])
  expect(ownWords(run('pending'))).toBe('HQ is working on it')
  expect(ownWords(run('answered'))).toBe('Done by HQ')
  expect(ownTitle(run('pending'))).toBe('Fix login')
  expect(ownCardId(run('pending'))).toBe('edge:k1')
  const page = src('routes', 'helpers.tsx')
  expect(page).toContain("t('helpers_own_work')")
})

test('Stop every helper counts running helpers per computer and shows a radio group', () => {
  expect(runningByHost([make({}), make({ id: 'x' }), make({ id: 'y', host: 'far' }), make({ id: 'z', status: 'exited' })])).toEqual({ native: 2, far: 1 })
  const source = src('components', 'sessions', 'EmergencyStop.tsx')
  expect(source).toContain('type="radio"')
  expect(source).not.toContain('<select')
  expect(t('helpers_stop_all_cant')).toBe("Can't be reached right now. Its helpers can't be stopped from here.")
})

test('the task filter says which task it shows and offers Show all', () => {
  const list = [make({ task: { id: 't1', display_id: 'T-9', title: 'Ship it', status: 'x' } })]
  const banner = taskBanner(list, 't1')
  expect(banner.title).toBe('Helpers on T-9')
  expect(banner.line).toBe('Showing the helpers on T-9 Ship it.')
  expect(t('helpers_show_all')).toBe('Show all')
})

test('Start a helper asks in the spec order and says the footer promise', () => {
  const source = src('components', 'sessions', 'NewAgentDialog.tsx')
  const order = ['helpers_new_first', 'helpers_new_which', 'helpers_new_name', 'helpers_new_where', 'helpers_new_folder', 'helpers_new_advanced'].map((k) => source.indexOf(k))
  expect(order).toEqual([...order].sort((a, b) => a - b))
  expect(order.every((i) => i > -1)).toBe(true)
  expect(t('helpers_new_asks')).toBe('It asks before it runs commands or changes files. You answer each step here and in your Inbox.')
  // WR17: a task is sent only when the dialog was opened from a task page.
  expect(source).toContain('...(task ? { task_id: task.id } : {})')
})

test('the list takes the whole page below 1024 and the detail shows Back to Helpers', () => {
  expect(src('routes', 'helpers.tsx')).toContain('min-[1024px]:flex')
  expect(renderToStaticMarkup(createElement(SessionDetail, { session: make({}), onBack() {}, onChanged() {} }))).toContain('Back to Helpers')
})
