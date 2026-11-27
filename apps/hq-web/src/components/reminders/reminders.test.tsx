import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { NotificationDetail } from '~/components/notifications/NotificationDetail'
import type { NotificationItem } from '~/lib/notificationsApi'
import { t } from '~/strings'
import { RemindersPage } from './RemindersPage'

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const router = createRouter({
    routeTree: createRootRoute({ component: node }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  return renderToStaticMarkup(createElement(QueryClientProvider, { client: new QueryClient() }, createElement(RouterProvider, { router })))
}

const ping = (over: Partial<NotificationItem> = {}): NotificationItem => ({
  id: 'val_r1',
  kind: 'value_item',
  category: 'value',
  title: 'Call Moses about the 50,000',
  description: 'Due 4:00 pm today.',
  state: 'pending',
  created_at: new Date().toISOString(),
  metadata: { extra: { source_task: 'reminder' }, link: '/reminders?id=abc' },
  actions: ['done', 'snooze', 'dismiss'],
  ...over,
})

const detail = (n: NotificationItem) =>
  inRouter(() => createElement(NotificationDetail, { notification: n, onAction: async () => undefined, onAnswered: () => undefined, loadingAction: null }))

describe('a reminder in the Inbox', () => {
  test('its ping offers Done and the snooze times, and Clear stays', async () => {
    const html = await detail(ping())
    expect(html).toContain('data-testid="reminder-actions"')
    expect(html).toContain(`>${t('reminders_mark_done')}<`)
    expect(html).toContain(`>${t('reminders_snooze_hour')}<`)
    expect(html).toContain(`>${t('reminders_snooze_tomorrow')}<`)
    expect(html).toContain(`>${t('inbox_clear')}<`)
  })

  test('the buttons name the reminder for a screen reader', async () => {
    const html = await detail(ping())
    expect(html).toContain(`aria-label="${t('reminders_done_aria', { title: 'Call Moses about the 50,000' })}"`)
  })

  test('an update that is not a reminder gets no Done or Snooze', async () => {
    const html = await detail(ping({ actions: ['dismiss', 'acknowledge'], metadata: {} }))
    expect(html).not.toContain('reminder-actions')
    expect(html).not.toContain(`>${t('reminders_mark_done')}<`)
  })

  test('a reminder already answered has no buttons', async () => {
    const html = await detail(ping({ state: 'approved', actions: [] }))
    expect(html).not.toContain('reminder-actions')
  })
})

describe('the Reminders page', () => {
  test('it has a title, a New reminder button and the two tabs', async () => {
    const html = await inRouter(() => createElement(RemindersPage))
    expect(html).toContain(`>${t('reminders_title')}<`)
    expect(html).toContain(t('reminders_new'))
    expect(html).toContain(`>${t('reminders_tab_upcoming')}<`)
    expect(html).toContain(`>${t('reminders_tab_done')}<`)
    expect(html).toContain('role="tablist"')
  })
})
