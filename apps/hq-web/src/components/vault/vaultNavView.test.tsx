import { describe, expect, test } from 'bun:test'
import { createElement, type ReactElement } from 'react'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { renderToStaticMarkup } from 'react-dom/server'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { FolderTree, LocationBar } from './VaultNav'

/** Renders a view that holds router Links, inside a router with one route. */
async function inRouter(view: ReactElement): Promise<string> {
  const router = createRouter({
    routeTree: createRootRoute({ component: () => view }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const noop = () => {}
const file = (path: string): NoteTreeNode => ({ name: path.split('/').pop() as string, path, type: 'file' })
const dir = (path: string, children: NoteTreeNode[]): NoteTreeNode => ({ name: path.split('/').pop() as string, path, type: 'dir', children })

const notes = (n: number) => Array.from({ length: n }, (_, i) => file(`Notebooks/Projects/Shop/Note ${String(i + 1).padStart(2, '0')}.md`))
const TREE = dir('Notebooks', [
  dir('Notebooks/_system', [file('Notebooks/_system/rules.md')]),
  dir('Notebooks/Daily', [file('Notebooks/Daily/2026-11-17.md')]),
  dir('Notebooks/Projects', [dir('Notebooks/Projects/Shop', notes(30)), dir('Notebooks/Projects/Academy', [])]),
])

const bar = (over: Partial<Parameters<typeof LocationBar>[0]> = {}) =>
  inRouter(createElement(LocationBar, { path: '', isFile: false, treeOpen: true, treeAvailable: true, onToggleTree: noop, ...over }))

describe('the location bar', () => {
  test('on Vault home Up is dimmed and the only part is Vault, as the current page', async () => {
    const html = await bar()
    expect(html).toContain('aria-disabled="true"')
    expect(html).toContain('aria-current="page"')
    expect(html).not.toContain('href="/notes/Notebooks')
  })

  test('in a folder every part above is a link, the folder is the current page, and Up names the parent', async () => {
    const html = await bar({ path: 'Notebooks/Projects/Shop' })
    expect(html).toContain('aria-label="Up to Projects"')
    expect(html).toContain('href="/notes/Notebooks/Projects"')
    expect(html).toMatch(/aria-current="page"[^>]*>Shop</)
    expect(html).toContain('aria-label="Where you are in the Vault"')
  })

  test('from a top-level folder Up goes to Vault', async () => {
    const html = await bar({ path: 'Notebooks/Projects' })
    expect(html).toContain('aria-label="Up to Vault"')
    expect(html).toContain('href="/notes"')
  })

  test('a note ends in its own name, and Up goes to its folder', async () => {
    const html = await bar({ path: 'Notebooks/Projects/Shop/Stock rules.md', isFile: true })
    expect(html).toContain('aria-label="Up to Shop"')
    expect(html).toMatch(/aria-current="page"[^>]*>Stock rules</)
  })

  test('a deep path folds its middle into one menu button', async () => {
    const html = await bar({ path: 'Notebooks/A/B/C/D/E', isFile: false })
    expect(html).toContain('aria-label="Folders above"')
    expect(html).toContain('aria-haspopup="menu"')
  })

  test('the folders button says whether the tree is open', async () => {
    expect(await bar({ treeOpen: true })).toContain('aria-pressed="true"')
    expect(await bar({ treeOpen: false })).toContain('aria-pressed="false"')
    expect(await bar({ treeAvailable: false })).not.toContain('vn-toggle')
  })

  test('the Trash is a part under Vault, not a folder', async () => {
    const html = await bar({ extra: 'Trash' })
    expect(html).toMatch(/aria-current="page"[^>]*>Trash</)
  })
})

describe('the folder tree', () => {
  const tree = (over: Partial<Parameters<typeof FolderTree>[0]> = {}) =>
    inRouter(
      createElement(FolderTree, {
        root: TREE,
        loading: false,
        failed: false,
        current: 'Notebooks/Projects/Shop',
        open: new Set(['Notebooks/Projects', 'Notebooks/Projects/Shop']),
        showAll: new Set<string>(),
        onToggle: noop,
        onShowAll: noop,
        ...over,
      }),
    )

  test('is a tree with levels, an open folder, a selected row and the path to it marked', async () => {
    const html = await tree()
    expect(html).toContain('role="tree"')
    expect(html).toContain('aria-level="1"')
    expect(html).toContain('aria-level="3"')
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toMatch(/aria-selected="true"[^>]*data-path="Notebooks\/Projects\/Shop"/)
    expect(html).toContain('is-path')
  })

  test('names a folder with its note count', async () => {
    const html = await tree()
    expect(html).toContain('aria-label="Shop, 30 notes"')
    expect(html).toContain('aria-label="Daily, 1 notes"')
  })

  test('leaves an underscore folder out', async () => {
    expect(await tree()).not.toContain('_system')
  })

  test('only one row is a tab stop', async () => {
    const html = await tree()
    expect(html.match(/tabindex="0"/g)).toHaveLength(1)
  })

  test('a long folder stops at 25 and offers Show all with its total', async () => {
    const html = await tree()
    expect(html).toContain('See all 30')
    const shown = (html.match(/>Note \d\d</g) ?? []).length
    expect(shown).toBe(25)
  })

  test('has the Trash at its foot', async () => {
    expect(await tree()).toContain('href="/notes/trash"')
  })

  test('says so when it is loading or cannot load', async () => {
    expect(await tree({ root: undefined, loading: true })).toContain('role="status"')
    expect(await tree({ root: undefined, failed: true })).toContain('Couldn&#x27;t load the folders.')
  })
})
