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
    similar: 0,
    similar_checked: 0,
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
  similar: [],
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

const health = (r: HealthReport | null, checked = true, over: Partial<VaultHealth> = {}): VaultHealth => ({
  checked,
  checked_at: checked ? new Date(Date.now() - 5 * 60_000).toISOString() : null,
  report: r,
  fixes: [],
  fix_counts: null,
  skipped: [],
  repairs: [],
  trend: [],
  ...over,
})

const repairRow = (over: Partial<VaultHealth['repairs'][number]> = {}) => ({
  run_id: 'r1',
  ran_at: new Date(Date.now() - 30 * 60_000).toISOString(),
  source: 'owner' as const,
  fixes: 3,
  rewritten: 2,
  trashed: 0,
  undone_at: null,
  edited_since: 0,
  ...over,
})

const aFix = (over: Partial<NonNullable<VaultHealth['fixes']>[number]> = {}) => ({
  id: 'f1',
  tier: 'a' as const,
  kind: 'stale_path' as const,
  note: 'Notebooks/Work/A.md',
  line: 1,
  before: '[[Gone 0]]',
  after: '[[Here]]',
  ...over,
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

describe('fixing from the card', () => {
  const dirty = (n = 3) => health(deadReport(12), true, { fix_counts: { a: n, b: 1 }, fixes: [aFix()] })

  test('offers a Fix button with the number of sure fixes, and only when it can act on it', async () => {
    const on = await inRouter(<VaultHealthCardView health={dirty(3)} onFix={() => {}} />)
    expect(on).toContain('Fix 3 things')
    expect(await inRouter(<VaultHealthCardView health={dirty(1)} onFix={() => {}} />)).toContain('Fix 1 thing')
    expect(await inRouter(<VaultHealthCardView health={dirty(0)} onFix={() => {}} />)).not.toContain('>Fix ')
    expect(await inRouter(<VaultHealthCardView health={dirty(3)} />)).not.toContain('Fix 3')
  })

  test('says it is working while a fix runs', async () => {
    expect(await inRouter(<VaultHealthCardView health={dirty(3)} busy onFix={() => {}} />)).toContain('Fixing')
  })

  test('after a fix that left nothing wrong it stays for a day, with Undo, and then goes quiet', async () => {
    const fixed = health(report(), true, { repairs: [repairRow()] })
    const html = await inRouter(<VaultHealthCardView health={fixed} onUndo={() => {}} />)
    expect(html).toContain('HQ fixed 3 things in your notes')
    expect(html).toContain('Undo')
    const old = health(report(), true, { repairs: [repairRow({ ran_at: new Date(Date.now() - 3 * 86_400_000).toISOString() })] })
    expect(renderToStaticMarkup(<VaultHealthCardView health={old} onUndo={() => {}} />)).toBe('')
    const taken = health(report(), true, { repairs: [repairRow({ undone_at: 'x' })] })
    expect(renderToStaticMarkup(<VaultHealthCardView health={taken} onUndo={() => {}} />)).toBe('')
  })

  test('with problems and a recent fix it offers Undo beside what is left', async () => {
    const both = health(deadReport(2), true, { repairs: [repairRow()], fix_counts: { a: 0, b: 0 } })
    const html = await inRouter(<VaultHealthCardView health={both} onUndo={() => {}} />)
    expect(html).toContain('2 links lead to notes that are not there')
    expect(html).toContain('HQ fixed 3 things in your notes')
  })
})

describe('fixing from the review page', () => {
  const noop = () => {}
  const view = (h: VaultHealth, over: Partial<Parameters<typeof VaultHealthView>[0]> = {}) =>
    inRouter(
      <VaultHealthView health={h} loadFailed={false} scanning={false} busyKey={null} onScan={noop} onIgnore={noop} onFix={noop} onFixAll={noop} onUndo={noop} {...over} />,
    )

  test('shows the repair under the item it is for, with how sure HQ is, and a Fix button', async () => {
    const h = health(deadReport(1), true, { fixes: [aFix()], fix_counts: { a: 1, b: 0 } })
    const html = await view(h)
    expect(html).toContain('data-health-fix')
    expect(html).toContain('Sure')
    expect(html).toContain('Becomes [[Here]]')
    expect(html).toContain('>Fix<')
  })

  test('a likely repair says so', async () => {
    const h = health(deadReport(1), true, { fixes: [aFix({ tier: 'b', kind: 'near_miss' })], fix_counts: { a: 0, b: 1 } })
    expect(await view(h)).toContain('Likely')
  })

  test('an item with no repair shows no Fix button, and there is no bulk button when nothing is sure', async () => {
    const h = health(deadReport(1), true, { fixes: [], fix_counts: { a: 0, b: 0 } })
    const html = await view(h)
    expect(html).not.toContain('>Fix<')
    expect(html).not.toContain('Fix 0')
  })

  test('offers to fix every sure one at once', async () => {
    const h = health(deadReport(2), true, { fixes: [aFix()], fix_counts: { a: 5, b: 0 } })
    expect(await view(h)).toContain('Fix 5 things')
  })

  test('says which notes were left alone and why', async () => {
    const h = health(deadReport(1), true, {
      skipped: [
        { note: 'Notebooks/Sheet.md', why: 'suite' },
        { note: 'Notebooks/Mine.md', why: 'pinned' },
      ],
    })
    const html = await view(h)
    expect(html).toContain('2 notes were left alone')
    expect(html).toContain('A sheet, deck or dashboard')
    expect(html).toContain('Pinned, so only you change it')
  })

  test('lists recent fixes with Undo, and says when one was taken back', async () => {
    const h = health(report(), true, { repairs: [repairRow(), repairRow({ run_id: 'r0', undone_at: 'x', source: 'auto' })] })
    const html = await view(h)
    expect(html).toContain('Recent fixes')
    expect(html).toContain('3 fixed.')
    expect((html.match(/data-health-run/g) ?? []).length).toBe(2)
    expect(html).toContain('Taken back')
    expect(html).toContain('Done by HQ')
    expect((html.match(/>Undo</g) ?? []).length).toBe(1)
  })

  test('near repeats are listed but are not counted as something wrong', async () => {
    const r = report({ similar: [{ a: 'Notebooks/A.md', b: 'Notebooks/B.md', percent: 90 }] }, {}, { similar: 1, similar_checked: 30 })
    const html = await view(health(r))
    expect(html).toContain('1 pair of notes says almost the same thing')
    expect(html).toContain('Every link reaches a note')
  })
})

