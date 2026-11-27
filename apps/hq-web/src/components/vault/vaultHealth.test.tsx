import { describe, expect, test } from 'bun:test'
import { createElement, type ReactElement } from 'react'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { renderToStaticMarkup } from 'react-dom/server'
import { VaultHealthCardView } from './VaultHealthCard'
import { VaultHealthView } from '~/routes/notes/health'
import type { HealthReport, VaultHealth } from '~/lib/vaultHealthApi'

const report = (over: Partial<HealthReport> = {}, links: Partial<HealthReport['counts']['links']> = {}, rest: Partial<HealthReport['counts']> = {}): HealthReport => ({
  counts: {
    notes: 20,
    unread: 0,
    links: { total: 60, ok: 55, dead: 0, ambiguous: 0, broken_anchors: 0, not_checked: 5, ...links },
    loose: 0,
    duplicates: 0,
    empty: 0,
    frontmatter: 0,
    ...rest,
  },
  dead_links: [],
  ambiguous_links: [],
  broken_anchors: [],
  loose: [],
  duplicates: [],
  empty: [],
  frontmatter: [],
  unread: [],
  ...over,
})

/** Renders a view that holds router Links, inside a router with one route. */
async function inRouter(view: ReactElement): Promise<string> {
  const router = createRouter({
    routeTree: createRootRoute({ component: () => view }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const health = (r: HealthReport | null, checked = true): VaultHealth => ({
  checked,
  checked_at: checked ? new Date(Date.now() - 5 * 60_000).toISOString() : null,
  report: r,
  trend: [],
})

const deadReport = (n: number) =>
  report(
    { dead_links: Array.from({ length: Math.min(n, 50) }, (_, i) => ({ note: 'Notebooks/Work/A.md', line: i + 1, link: `[[Gone ${i}]]`, target: `Gone ${i}`, did_you_mean: null })) },
    { dead: n },
  )

describe('VaultHealthCardView', () => {
  test('says nothing before a check, after a clean one, or when the server cannot answer', () => {
    expect(renderToStaticMarkup(<VaultHealthCardView health={undefined} />)).toBe('')
    expect(renderToStaticMarkup(<VaultHealthCardView health={health(null, false)} />)).toBe('')
    expect(renderToStaticMarkup(<VaultHealthCardView health={health(report())} />)).toBe('')
  })

  test('notes that are only loose or empty are not a reason to speak up', () => {
    expect(renderToStaticMarkup(<VaultHealthCardView health={health(report({}, {}, { loose: 9, empty: 2 }))} />)).toBe('')
  })

  test('names the count and what is wrong, in the right plural', async () => {
    const one = await inRouter(<VaultHealthCardView health={health(deadReport(1))} />)
    expect(one).toContain('1 thing in your notes needs a look')
    expect(one).toContain('1 link leads to a note that is not there')
    const many = await inRouter(<VaultHealthCardView health={health(deadReport(12))} />)
    expect(many).toContain('12 things in your notes need a look')
    expect(many).toContain('12 links lead to notes that are not there')
    expect(many).toContain('Review')
  })
})

describe('VaultHealthView', () => {
  const noop = () => {}
  const view = (h: VaultHealth | undefined, over: Partial<Parameters<typeof VaultHealthView>[0]> = {}) =>
    inRouter(<VaultHealthView health={h} loadFailed={false} scanning={false} busyKey={null} onScan={noop} onIgnore={noop} {...over} />)

  test('before any check it says so and offers one', async () => {
    const html = await view(health(null, false))
    expect(html).toContain('HQ has not checked your notes yet.')
    expect(html).toContain('Check now')
  })

  test('a clean check says so plainly', async () => {
    const html = await view(health(report()))
    expect(html).toContain('Every link reaches a note. Nothing needs fixing.')
    expect(html).toContain('Checked 5 min ago')
  })

  test('lists each thing with where it is, and the exact total when examples are capped', async () => {
    const html = await view(health(deadReport(120)))
    expect(html).toContain('120 links lead to notes that are not there')
    expect(html).toContain('[[Gone 0]]')
    expect(html).toContain('Work')
    expect(html).toContain('Line 1')
    expect(html).toContain('and 70 more')
    expect((html.match(/data-health-item/g) ?? []).length).toBe(50)
  })

  test('a check in progress and a failed load are each said', async () => {
    expect(await view(health(report()), { scanning: true })).toContain('Checking')
    expect(await view(undefined, { loadFailed: true })).toContain('Could not load the note check.')
  })
})
