import { expect, test } from 'bun:test'
import { RouterProvider, createMemoryHistory, createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { Panes } from '~/components/ui/Panes'
import type { HarnessSession } from '~/lib/sessionsApi'
import { NewAgentDialog } from './NewAgentDialog'
import { StopAllDialog } from './EmergencyStop'
import { SessionInspector } from './SessionInspector'
import { SessionRow } from './SessionRow'

const make = (over: Record<string, unknown>) =>
  ({
    id: 'hs-1', harness: 'claude', label: 'Gradle', host: 'native', cwd: '/work/app', status: 'running', agent_status: 'working',
    created_at: '2025-01-01 10:00:00', last_seen_at: '2025-01-01 10:14:00', archived: false, task: null, owner_thread: null,
    pending: null, pending_seq: null, ...over,
  }) as unknown as HarnessSession

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const root = createRootRoute({ component: node })
  const detail = createRoute({ getParentRoute: () => root, path: '/helpers/$id', component: () => null })
  const router = createRouter({ routeTree: root.addChildren([detail]), history: createMemoryHistory({ initialEntries: ['/'] }) })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const here = import.meta.dir
const read = (...p: string[]) => readFileSync(join(here, ...p), 'utf8')

test('the Helpers page is three panes', () => {
  expect(read('..', '..', 'routes', 'helpers.tsx')).toContain('<Panes')
  const html = renderToStaticMarkup(createElement(Panes, { list: 'l', detail: 'd', side: createElement(SessionInspector, { session: make({}) }), showDetail: true }))
  expect(html).toContain('a-panes')
  expect(html).toContain('a-pane-list')
  expect(html).toContain('a-pane-detail')
  expect(html).toContain('a-pane-side')
})

test('a waiting helper is a needs card with its state and answer buttons', async () => {
  const pending = {
    kind: 'command', question: null, command: './gradlew build', truncated: false,
    options: [{ id: 1, label: 'Yes', remember: false, typed: false, selected: true }, { id: 2, label: 'No', remember: false, typed: false, selected: false }],
  }
  const html = await inRouter(() => createElement(SessionRow, { session: make({ agent_status: 'blocked', pending, pending_seq: 3 }), selected: false, onAnswered() {} }))
  expect(html).toContain('a-card needs')
  expect(html).toContain('a-state')
  expect(html).toContain('a-card-actions')
})

test('the details pane is a column of rows', () => {
  const html = renderToStaticMarkup(createElement(SessionInspector, { session: make({}) }))
  expect(html).toContain('a-drow')
})

test('Start a helper and Stop every helper are sheets, not hand-made overlays', () => {
  const start = renderToStaticMarkup(createElement(NewAgentDialog, { onClose() {}, onStarted() {} }))
  const stop = renderToStaticMarkup(createElement(StopAllDialog, { onClose() {}, onStopped() {} }))
  for (const html of [start, stop]) {
    expect(html).toContain('a-sheet')
    expect(html).not.toContain('fixed inset-0')
  }
})

test('the Helpers files carry none of the old look', () => {
  const dir = join(here)
  const files = readdirSync(dir)
    .filter((f) => /^[A-Za-z]+\.tsx?$/.test(f) && !f.includes('.test.'))
    .map((f) => join('components', 'sessions', f))
    .concat(join('routes', 'helpers.tsx'))
  for (const f of files) {
    const text = read('..', '..', f)
    for (const bad of ['emerald', 'rose', '--accent-red', 'hq-row', 'hq-card', 'hq-composer']) expect([f, text.includes(bad)]).toEqual([f, false])
  }
})
