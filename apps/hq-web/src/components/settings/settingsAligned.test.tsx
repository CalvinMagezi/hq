import { expect, test } from 'bun:test'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { ProviderKeyRow } from '~/lib/settingsApi'
import type { BudgetStatus } from '~/lib/usageApi'
import { KeysView } from './ProviderKeysSection'
import { SettingsHome } from './SettingsHome'
import { LimitRow } from './UsageLimits'

const here = import.meta.dir
const read = (p: string) => readFileSync(join(here, p), 'utf8')

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const root = createRootRoute({ component: node })
  const router = createRouter({ routeTree: root, history: createMemoryHistory({ initialEntries: ['/'] }) })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const row = (o: Record<string, unknown>) => ({ name: 'X', configured: true, ...o }) as unknown as ProviderKeyRow
const keys = (editing: string | null) =>
  createElement(KeysView, { rows: [row({ name: 'Anthropic', id: 'anthropic', configured: false })], canEdit: true, reachable: true, editing, saved: null, onEdit: () => {}, onSaved: () => {}, onCancel: () => {} })

test('Settings is panes, and its home is grouped list rows with icons', async () => {
  expect(read('../../routes/settings.tsx')).toContain('<Panes')
  const html = await inRouter(() =>
    createElement(SettingsHome, { lite: false, hub: false, trailing: {}, onSignOut: () => {} } as never),
  )
  expect(html).toContain('a-grp')
  expect(html).toContain('a-li')
  expect(html).toContain('a-ic')
})

test('keys are one set of rows, not a card each, and the sheet is the shared one', () => {
  const list = renderToStaticMarkup(keys(null))
  expect(list).toContain('a-set')
  expect(list).toContain('a-li')
  expect(list).not.toContain('a-card')
  const sheet = renderToStaticMarkup(keys('anthropic'))
  expect(sheet).toContain('a-sheet')
  expect(sheet).toContain('a-field')
  expect(sheet).toContain('a-input')
})

test('a spending limit is a list row with an 8px meter', async () => {
  const b: BudgetStatus = { name: 'Daily', scope: 'global', period: 'daily', action: 'warn', limit_usd: 10, spent_usd: 4, pct: 40, state: 'ok', period_start: 0, resets_at: Date.now() + 3_600_000 } as BudgetStatus
  const html = await inRouter(() => createElement(LimitRow, { b, def: undefined, away: false, onEdit: () => {}, onDelete: () => {} }))
  expect(html).toContain('a-li')
  expect(html).toContain('a-prog')
  expect(html).toContain('st-meter')
  expect(html).not.toContain('a-card')
  expect(read('../../../app.css')).toMatch(/\.st-meter[^{]*\{height:8px/)
})

test('no settings file keeps old colours, glass or the claude.ai connector', () => {
  const files = readdirSync(here).filter((f) => /\.tsx$/.test(f) && !f.includes('test')).map((f) => read(f))
  files.push(read('../../routes/settings.tsx'))
  for (const src of files) {
    expect(src).not.toMatch(/emerald|rose-|amber-|neutral-|hq-btn-ghost|hq-card|rgba\(|fixed inset-0|claude\.ai|Claude on the web/)
  }
})
