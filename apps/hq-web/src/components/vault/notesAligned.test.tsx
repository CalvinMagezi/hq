import { describe, expect, test } from 'bun:test'
import { readFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { createElement, type ReactElement } from 'react'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { renderToStaticMarkup } from 'react-dom/server'
import { NotesHomeView } from '~/routes/notes/index'
import { FolderView, NoteBar, NoteLayout } from './NoteViews'
import { applyFormat } from '../NoteEditor'
import { folderChoices, folderTrail, countNotes } from './noteRows'

const SRC = join(import.meta.dir, '..', '..')
const read = (rel: string) => readFileSync(join(SRC, rel), 'utf8')
const noop = () => {}

/** Renders a view that holds router Links, inside a router with one route. */
async function inRouter(view: ReactElement): Promise<string> {
  const router = createRouter({
    routeTree: createRootRoute({ component: () => view }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const OWNED = [
  'routes/notes.tsx',
  'routes/notes/index.tsx',
  'routes/notes/$.tsx',
  'routes/notes/n.$id.tsx',
  'components/NoteEditor.tsx',
  'components/QuickNoteOverlay.tsx',
  'components/ExportMenu.tsx',
  'components/CopyPathMenu.tsx',
  'components/notes/AboutNote.tsx',
  'components/notes/ReviewTag.tsx',
  'components/vault/HomeSection.tsx',
  'components/vault/HomeSections.tsx',
  'components/vault/PinnedCard.tsx',
  'components/vault/GetStartedCard.tsx',
  'components/vault/NoteViews.tsx',
  'components/vault/notes.css',
]

describe('Notes on the aligned boards', () => {
  test('the tree sidebar is gone and the layout is only an outlet', () => {
    const layout = read('routes/notes.tsx')
    expect(layout).not.toContain('VaultSidebar')
    expect(layout).toContain('Outlet')
    for (const gone of ['VaultSidebar.tsx', 'VaultTree.tsx', 'useSidebarResize.ts']) {
      expect(existsSync(join(SRC, 'components/vault', gone))).toBe(false)
    }
  })

  test('home is find, folders, pinned and recent, with no task list or activity block', () => {
    const html = renderToStaticMarkup(
      createElement(NotesHomeView, {
        pinned: { kind: 'empty' },
        recent: { kind: 'empty' },
        folders: [],
        onRetryPinned: noop,
        onRetryRecent: noop,
      }),
    )
    expect(html).toContain('a-panes')
    expect(html).toContain('an-home')
    expect(html).toContain('an-findpill')
    expect(html).not.toContain('In Progress Tasks')
    expect(html).not.toContain('System Activity')
    expect(html).not.toContain('glass-card')
    expect(html).toMatch(/an-findpill[^>]*>\s*<svg[\s\S]*?<span>[\s\S]*?<kbd class="a-kbd"/)
    expect(read('routes/notes/index.tsx')).not.toContain('GetStartedCard')
  })

  test('HQ Lite has no Ask HQ and no Export on a note (NotesNote2)', () => {
    const bar = (lite: boolean) =>
      renderToStaticMarkup(
        createElement(NoteBar, { editing: false, pinned: false, pinning: false, exporting: false, exportOpen: false, onEdit: noop, onAsk: noop, onTask: noop, onPin: noop, onExport: noop, lite }),
      )
    expect(bar(false)).toContain('Ask HQ')
    expect(bar(false)).toContain('Export')
    expect(bar(true)).not.toContain('Ask HQ')
    expect(bar(true)).not.toContain('Export')
    expect(bar(true)).toContain('Make a task')
  })

  test('a folder lists its notes as rows, not emoji tiles', async () => {
    const html = await inRouter(
      createElement(FolderView, {
        dirPath: 'Notebooks/Work',
        entries: [{ name: 'Plan.md', path: 'Notebooks/Work/Plan.md', isDir: false }],
        onNewHere: noop,
      }),
    )
    expect(html).toContain('a-row')
    expect(html).toContain('an-crumbs')
    expect(html).not.toContain('📁')
  })

  test('the note bar has Edit, Ask HQ, Make a task, Pin and Export; Copy link is in More', () => {
    const html = renderToStaticMarkup(
      createElement(NoteBar, { editing: false, pinned: true, pinning: false, exporting: false, exportOpen: false, onEdit: noop, onAsk: noop, onTask: noop, onPin: noop, onExport: noop }),
    )
    expect(html).toContain('an-bar')
    for (const word of ['Edit', 'Ask HQ', 'Make a task', 'Unpin', 'Export']) expect(html).toContain(word)
    expect(html).toContain('aria-pressed="true"')
    expect(html).not.toContain('Copy link')
    expect(read('components/CopyPathMenu.tsx')).toContain("t('notes_copy_link')")
  })

  test('About this note sits in the side pane', async () => {
    const html = await inRouter(
      createElement(NoteLayout, { filePath: 'Notebooks/Work/Plan.md', side: createElement('b', null, 'about'), sideOpen: true, onToggleSide: noop, onMore: noop, children: 'text' }),
    )
    expect(html).toMatch(/a-pane-side[^>]*>\s*<b>about<\/b>/)
    expect(html).toContain('side-open')
    expect(html).toContain('aria-haspopup="menu"')
  })

  test('a new note is a sheet with no native select', () => {
    const src = read('components/QuickNoteOverlay.tsx')
    expect(src).toContain('<Sheet')
    expect(src).not.toContain('<select')
    expect(src).toContain('menuitemradio')
  })

  test('no Tailwind colour, glass, blur or raw colour survives in what Notes owns', () => {
    for (const file of OWNED) {
      const src = read(file)
      for (const bad of ['glass-card', 'glass-heavy', 'emerald', 'violet', 'amber-', 'neutral-', 'rgba(', 'backdropFilter', '\u2014']) {
        if (file.endsWith('$.tsx') && bad === 'rgba(') continue
        expect([file, bad, src.includes(bad)]).toEqual([file, bad, false])
      }
    }
  })
})

describe('Notes helpers', () => {
  test('formatting wraps a selection or starts the line', () => {
    expect(applyFormat('a word b', 2, 6, 'bold').text).toBe('a **word** b')
    expect(applyFormat('one\ntwo', 4, 7, 'checklist').text).toBe('one\n- [ ] two')
  })

  test('folder trail and choices read from Notebooks', () => {
    expect(folderTrail('Notebooks/Work/Plan.md')).toBe('Work')
    expect(folderChoices(['Notebooks', 'Notebooks/Work', 'Notebooks/Work/Deep']).map((f) => f.depth)).toEqual([0, 1])
  })

  test('countNotes counts markdown at any depth', () => {
    const tree = { name: 'a', path: 'a', type: 'dir' as const, children: [{ name: 'x.md', path: 'a/x.md', type: 'file' as const }, { name: 'b', path: 'a/b', type: 'dir' as const, children: [{ name: 'y.md', path: 'a/b/y.md', type: 'file' as const }, { name: 'z.png', path: 'a/b/z.png', type: 'file' as const }] }] }
    expect(countNotes(tree)).toBe(2)
  })
})
