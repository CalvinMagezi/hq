import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import type { NotificationItem } from '~/lib/notificationsApi'
import { t } from '~/strings'
import { InboxView, type InboxViewProps } from './InboxView'
import { NotificationCard } from './NotificationCard'
import { NotificationDetail } from './NotificationDetail'
import { countByKind, updateKind } from './updateKind'

const root = join(import.meta.dir, '..', '..', '..')

const item = (over: Partial<NotificationItem> & { id: string }): NotificationItem => ({
  kind: 'action_needed',
  category: 'system',
  title: 'HQ needs your OK',
  description: 'Delete 3 old backups.',
  state: 'pending',
  created_at: new Date().toISOString(),
  metadata: {},
  actions: ['approve', 'reject'],
  ...over,
})

const approval = item({
  id: 'a1',
  metadata: { approval: { tool: 'bash', args: { command: 'rm old.tar' }, loses: true, verb: 'delete' } },
})
const taskUpdate = item({ id: 'u1', kind: 'value_item', title: 'A task is ready to check', actions: ['dismiss'], metadata: { extra: { source_task: 'task_ready_for_review' } } })
const taskCheck = item({ id: 't1', title: 'GARDEN-004: Write the README', description: 'A long task description.', metadata: { extra: { source_task: 'task_ready_for_review' }, link: '/tasks?task=abc123' } })
const hqUpdate = item({ id: 'u2', kind: 'value_item', title: 'Backup finished', actions: ['dismiss'] })

const base: InboxViewProps = {
  tab: 'pending',
  onTab: () => undefined,
  items: [],
  needsCount: 0,
  updatesCount: 0,
  query: '',
  onQuery: () => undefined,
  filter: 'all',
  onFilter: () => undefined,
  selected: null,
  checkedAt: null,
  refreshing: false,
  onRefresh: () => undefined,
  onAction: async () => undefined,
  onClear: () => undefined,
  onAnswered: () => undefined,
  loadingAction: null,
  onOpenLink: () => undefined,
}

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const router = createRouter({
    routeTree: createRootRoute({ component: node }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  return renderToStaticMarkup(createElement(QueryClientProvider, { client: new QueryClient() }, createElement(RouterProvider, { router })))
}

const view = (over: Partial<InboxViewProps>) => inRouter(() => createElement(InboxView, { ...base, ...over }))

// web-matches-phone B5: the Inbox on the design's panes, cards and rows (AlignedInbox, AlignedInbox2).
describe('the Inbox panes', () => {
  test('a list and a detail pane, with the three tabs and their counts', async () => {
    const html = await view({ items: [approval], needsCount: 1, updatesCount: 4 })
    expect(html).toContain('class="a-panes hq-panes"')
    expect(html).toContain('a-pane a-pane-list')
    expect(html).toContain('a-pane a-pane-detail')
    expect(html).toContain('class="a-tabs"')
    expect(html).toContain('<em class="need">1</em>')
    expect(html).toContain('<em>4</em>')
    expect(html).toContain('class="a-search"')
  })

  test('the open item takes the whole width below 1024 and carries Back', async () => {
    const html = await view({ items: [approval], selected: approval })
    expect(html).toContain('a-panes hq-panes is-detail')
    expect(html).toContain('a-ib a-back')
  })
})

describe('Needs you cards', () => {
  test('an approval is an .a-card.needs with the quote, then Decline before the answer', async () => {
    const html = await view({ items: [approval], needsCount: 1 })
    expect(html).toContain('class="a-card needs"')
    expect(html).toContain('class="a-av a-warn-t"')
    expect(html).toContain('class="a-quote"')
    expect(html.indexOf(t('notice_action_decline'))).toBeGreaterThan(-1)
    expect(html.indexOf(t('notice_action_decline'))).toBeLessThan(html.indexOf('a-btn danger'))
    expect(html).toContain(t('approval_allow_verb', { verb: 'delete' }))
  })

  test('a plain notice follows the same card, with Clear as its one answer', async () => {
    const html = await inRouter(() =>
      createElement(NotificationCard, { notification: item({ id: 'n1', actions: ['dismiss'], description: 'Look it over.' }), onAction: async () => undefined, onAnswered: () => undefined, loadingAction: null }),
    )
    expect(html).toContain('a-card needs')
    expect(html).toContain('a-card-head')
    expect(html).toContain('Look it over.')
    expect(html).toContain(`>${t('inbox_clear')}<`)
  })

  test('a task to check offers Check it, which opens the task, and never Allow or Decline (Inbox board)', async () => {
    const html = await inRouter(() =>
      createElement(NotificationCard, { notification: taskCheck, onAction: async () => undefined, onAnswered: () => undefined, loadingAction: null }),
    )
    expect(html).toContain(`>${t('tasks_check_it')}<`)
    expect(html).toContain('href="/tasks?task=abc123"')
    expect(html).toContain(t('tasks_finished_plain'))
    expect(html).not.toContain(t('approval_quote_inbox'))
    expect(html).not.toContain('Allow once')
    expect(html).not.toContain('A long task description.')
  })

  test('a new sign-in offers It was me and a red Sign it out', async () => {
    const html = await inRouter(() =>
      createElement(NotificationCard, {
        notification: item({ id: 's1', metadata: { sign_in: { device_id: 'd1' } } }),
        onAction: async () => undefined,
        onAnswered: () => undefined,
        loadingAction: null,
      }),
    )
    expect(html).toContain(`<button type="button" class="a-btn secondary">${t('it_was_me')}`)
    expect(html).toContain(`<button type="button" class="a-btn danger">${t('sign_it_out')}`)
    expect(html).not.toContain('--ink')
  })
})

describe('Updates and Done', () => {
  test('an update is an .a-msg under a day .a-grp, with the four chips', async () => {
    const html = await view({ tab: 'all', items: [taskUpdate, hqUpdate], updatesCount: 2, onClearAll: () => undefined })
    expect(html.indexOf('class="a-grp"')).toBeGreaterThan(-1)
    expect(html.indexOf('class="a-grp"')).toBeLessThan(html.indexOf('class="a-msg"'))
    expect(html).toContain('a-msg-main')
    expect(html).toContain('a-msg-time')
    for (const chip of ['search_chip_all', 'search_chip_tasks', 'search_chip_helpers', 'inbox_chip_hq'] as const) expect(html).toContain(t(chip))
    expect(html).toContain(t('inbox_clear_all_updates'))
    expect(html).not.toContain('a-card')
  })

  test('the chip counts follow what the item already carries, and unclear items count under HQ', () => {
    const helper = item({ id: 'h', kind: 'value_item', metadata: { extra: { source_task: 'session-supervisor' } } })
    const linked = item({ id: 'l', kind: 'value_item', metadata: { link: '/tasks/t1' } })
    expect([taskUpdate, helper, linked, hqUpdate].map(updateKind)).toEqual(['tasks', 'helpers', 'tasks', 'hq'])
    expect(countByKind([taskUpdate, helper, linked, hqUpdate])).toEqual({ all: 4, tasks: 2, helpers: 1, hq: 1 })
  })

  test('a chip narrows the rows', async () => {
    const html = await view({ tab: 'all', items: [taskUpdate, hqUpdate], filter: 'tasks' })
    expect(html).toContain('A task is ready to check')
    expect(html).not.toContain('Backup finished')
  })
})

describe('caught up', () => {
  test('is an .a-empty with the catalogue words and the time it was checked', async () => {
    const html = await view({ checkedAt: new Date(2026, 10, 5, 14, 32) })
    expect(html).toContain('class="a-empty"')
    expect(html).toContain(t('inbox_you_re_all_caught').replace("'", '&#x27;'))
    expect(html).toContain('Nothing is waiting on you. Checked at')
  })

  test('says nothing before HQ has answered once', async () => {
    expect((await view({ checkedAt: null })).split('a-pane-detail')[0]).not.toContain('a-empty')
  })
})

describe('the open item', () => {
  const wait = (n: NotificationItem) => inRouter(() => createElement(NotificationDetail, { notification: n, onAction: async () => undefined, onAnswered: () => undefined, loadingAction: null }))

  test('details are .a-drow rows and the answers sit in a footer after the text', async () => {
    const html = await wait(approval)
    for (const key of ['inbox_d_asked_by', 'inbox_d_when', 'inbox_d_waits_until'] as const) expect(html).toContain(`<span>${t(key)}</span>`)
    expect(html.match(/class="a-drow"/g)?.length).toBe(3)
    expect(html.indexOf('class="a-quote"')).toBeLessThan(html.indexOf('hq-inbox-foot'))
    expect(html.indexOf(t('notice_action_decline'))).toBeLessThan(html.indexOf('a-btn danger'))
  })
})

describe('the files this item owns', () => {
  const dir = join(root, 'src', 'components', 'notifications')
  const files = [...readdirSync(dir).filter((f) => !f.includes('.test.')).map((f) => join(dir, f)), join(root, 'src', 'routes', 'inbox.tsx')]
  test('no Tailwind colours, old card classes or fixed overlays are left', () => {
    for (const file of files) {
      const text = readFileSync(file, 'utf8')
      for (const gone of ['emerald', 'purple', 'cyan', 'rose', 'amber-', 'neutral-', 'hq-card', 'rounded-2xl', 'fixed inset-0', 'glass']) {
        expect(text.includes(gone), `${file} has ${gone}`).toBe(false)
      }
    }
  })
})

describe('keeping the inbox: read, Done, silence', () => {
  const fresh = item({ id: 'n1', kind: 'value_item', title: 'Disk is filling up', actions: ['dismiss'], read: false, topic: 'upkeep' })
  const seen = item({ id: 'n2', kind: 'value_item', title: 'Backup finished', actions: ['dismiss'], read: true, topic: 'upkeep', silenced: true })

  test('an unread update is marked, a read one is not, and an older HQ marks nothing', async () => {
    const html = await view({ tab: 'all', items: [fresh, seen, hqUpdate], updatesCount: 3, onReadAll: () => undefined, onClearAll: () => undefined })
    expect(html.match(/data-unread/g)?.length).toBe(1)
    expect(html).toContain(t('inbox_read_all'))
    expect(html).toContain(t('inbox_clear_all_updates'))
    expect(html).toContain(`aria-label="${t('inbox_silenced')}"`)
  })

  test('Mark all as read shows only while something is unread, and Done can be cleared', async () => {
    const updates = await view({ tab: 'all', items: [seen], updatesCount: 1, onClearAll: () => undefined })
    expect(updates).not.toContain(t('inbox_read_all'))
    const done = await view({ tab: 'resolved', items: [item({ id: 'd1', state: 'approved', actions: [] })], onClearDone: () => undefined })
    expect(done).toContain(t('inbox_clear_done'))
    expect(await view({ tab: 'resolved', items: [] })).not.toContain(t('inbox_clear_done'))
  })

  test('only a topic can be silenced: what HQ asks itself never offers it', async () => {
    const helper = item({ id: 'h1', topic: 'helpers', title: "Harness session 'x' is waiting for you", actions: ['dismiss'] })
    const offered = await view({ tab: 'pending', items: [helper], selected: helper, onSilence: () => undefined })
    expect(offered).toContain(t('silence_this_helpers'))
    const own = { ...approval, topic: 'hq' }
    const html = await view({ tab: 'pending', items: [own], selected: own, onSilence: () => undefined })
    for (const key of ['silence_this_helpers', 'silence_this_tasks', 'silence_this_tips', 'silence_this_upkeep', 'silence_this_email'] as const) expect(html).not.toContain(t(key))
  })
})
