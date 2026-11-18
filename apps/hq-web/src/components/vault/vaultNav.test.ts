import { describe, expect, test } from 'bun:test'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { ancestorPaths, countNotesCached, existsInTree, findNode, foldTrail, furthestIndex, looksLikeFile, nearestFolder, siblingsOf, treeMove, treeRows, trailOf, upTarget, vaultLocation, type TreeRow } from './noteRows'

const file = (path: string): NoteTreeNode => ({ name: path.split('/').pop() as string, path, type: 'file' })
const dir = (path: string, children: NoteTreeNode[]): NoteTreeNode => ({ name: path.split('/').pop() as string, path, type: 'dir', children })

const TREE = dir('Notebooks', [
  dir('Notebooks/_system', [file('Notebooks/_system/rules.md')]),
  dir('Notebooks/Projects', [
    dir('Notebooks/Projects/Shop', [
      dir('Notebooks/Projects/Shop/Orders', [file('Notebooks/Projects/Shop/Orders/Q4.md')]),
      file('Notebooks/Projects/Shop/Stock rules.md'),
      file('Notebooks/Projects/Shop/Supplier terms.md'),
    ]),
    dir('Notebooks/Projects/Academy', [file('Notebooks/Projects/Academy/Plan.md')]),
  ]),
  dir('Notebooks/Daily', [file('Notebooks/Daily/2026-11-17.md')]),
])

describe('the location bar trail', () => {
  test('lists the folders above a path, shallowest first', () => {
    expect(ancestorPaths('Notebooks/Projects/Shop/Stock rules.md')).toEqual(['Notebooks', 'Notebooks/Projects', 'Notebooks/Projects/Shop'])
    expect(ancestorPaths('Notes.md')).toEqual([])
  })

  test('Vault home is just Vault', () => {
    expect(trailOf('', false)).toEqual([{ name: 'Vault', path: '' }])
  })

  test('a folder ends in itself, and the Notebooks root is left out', () => {
    expect(trailOf('Notebooks/Projects/Shop', false)).toEqual([
      { name: 'Vault', path: '' },
      { name: 'Projects', path: 'Notebooks/Projects' },
      { name: 'Shop', path: 'Notebooks/Projects/Shop' },
    ])
  })

  test('a note ends in its own name without .md', () => {
    const trail = trailOf('Notebooks/Projects/Shop/Stock rules.md', true)
    expect(trail.map((p) => p.name)).toEqual(['Vault', 'Projects', 'Shop', 'Stock rules'])
    expect(trail[3].path).toBe('Notebooks/Projects/Shop/Stock rules.md')
  })

  test('tells a file from a folder by its extension when the tree is not there yet', () => {
    expect(looksLikeFile('Notebooks/Plan.md')).toBe(true)
    expect(looksLikeFile('Notebooks/Projects')).toBe(false)
    expect(looksLikeFile('Notebooks/Projects/v1.2-notes')).toBe(false)
  })

  test('Up goes to the folder above, to Vault home from a top-level folder, and nowhere from home', () => {
    expect(upTarget('Notebooks/Projects/Shop/Stock rules.md')).toBe('Notebooks/Projects/Shop')
    expect(upTarget('Notebooks/Projects/Shop')).toBe('Notebooks/Projects')
    expect(upTarget('Notebooks/Projects')).toBe('')
    expect(upTarget('')).toBeNull()
  })

  test('a long trail folds its middle into one menu and keeps the first and the last two', () => {
    const short = foldTrail(['Vault', 'Projects', 'Shop', 'Orders'])
    expect(short).toEqual({ before: ['Vault', 'Projects', 'Shop', 'Orders'], hidden: [], after: [] })
    const long = foldTrail(['Vault', 'Projects', 'Shop', 'Orders', '2026', 'November', 'Quote'])
    expect(long.before).toEqual(['Vault'])
    expect(long.hidden).toEqual(['Projects', 'Shop', 'Orders', '2026'])
    expect(long.after).toEqual(['November', 'Quote'])
  })
})

describe('the folder tree rows', () => {
  const open = (...paths: string[]) => new Set(paths)

  test('shows folders only while everything is closed, and never an underscore folder', () => {
    const rows = treeRows(TREE, open())
    expect(rows.map((r) => r.name)).toEqual(['Daily', 'Projects'])
    expect(rows.every((r) => r.kind === 'folder' && r.depth === 0)).toBe(true)
  })

  test('an open folder lists its sub-folders first, then its notes A to Z', () => {
    const rows = treeRows(TREE, open('Notebooks/Projects', 'Notebooks/Projects/Shop'))
    expect(rows.map((r) => `${r.depth}:${r.name}`)).toEqual([
      '0:Daily',
      '0:Projects',
      '1:Academy',
      '1:Shop',
      '2:Orders',
      '2:Stock rules',
      '2:Supplier terms',
    ])
  })

  test('counts notes at any depth and leaves hidden folders out of the count', () => {
    expect(countNotesCached(TREE.children![1])).toBe(4)
    expect(countNotesCached(TREE)).toBe(5)
  })

  test('a folder with many notes stops at the limit and says how many are left', () => {
    const many = dir('Notebooks/Daily', Array.from({ length: 30 }, (_, i) => file(`Notebooks/Daily/2026-11-${String(i + 1).padStart(2, '0')}.md`)))
    const root = dir('Notebooks', [many])
    const rows = treeRows(root, open('Notebooks/Daily'), new Set(), 25)
    expect(rows.filter((r) => r.kind === 'note')).toHaveLength(25)
    const more = rows.find((r) => r.kind === 'more')
    expect(more && more.kind === 'more' && more.hiddenCount).toBe(5)
    expect(more && more.kind === 'more' && more.total).toBe(30)
    expect(more && more.kind === 'more' && more.folder).toBe('Notebooks/Daily')
    expect(more?.path).not.toBe('Notebooks/Daily')
    const all = treeRows(root, open('Notebooks/Daily'), new Set(['Notebooks/Daily']), 25)
    expect(all.filter((r) => r.kind === 'note')).toHaveLength(30)
    expect(all.some((r) => r.kind === 'more')).toBe(false)
  })

  test('an empty tree has no rows', () => {
    expect(treeRows(null, open())).toEqual([])
  })
})

describe('keys in the tree', () => {
  const rows: TreeRow[] = treeRows(TREE, new Set(['Notebooks/Projects', 'Notebooks/Projects/Shop']))
  const at = (name: string) => rows.findIndex((r) => r.name === name)

  test('arrows move between visible rows and stop at the ends', () => {
    expect(treeMove(rows, 0, 'ArrowDown')).toEqual({ focus: 1 })
    expect(treeMove(rows, 0, 'ArrowUp')).toBeNull()
    expect(treeMove(rows, rows.length - 1, 'ArrowDown')).toBeNull()
    expect(treeMove(rows, 3, 'Home')).toEqual({ focus: 0 })
    expect(treeMove(rows, 0, 'End')).toEqual({ focus: rows.length - 1 })
  })

  test('Right opens a closed folder, then steps into it', () => {
    expect(treeMove(rows, at('Academy'), 'ArrowRight')).toEqual({ toggle: 'Notebooks/Projects/Academy', open: true })
    expect(treeMove(rows, at('Shop'), 'ArrowRight')).toEqual({ focus: at('Shop') + 1 })
    expect(treeMove(rows, at('Stock rules'), 'ArrowRight')).toBeNull()
  })

  test('Left closes an open folder, then steps out to the parent', () => {
    expect(treeMove(rows, at('Shop'), 'ArrowLeft')).toEqual({ toggle: 'Notebooks/Projects/Shop', open: false })
    expect(treeMove(rows, at('Stock rules'), 'ArrowLeft')).toEqual({ focus: at('Shop') })
    expect(treeMove(rows, at('Daily'), 'ArrowLeft')).toBeNull()
  })
})

describe('finding where the person is', () => {
  test('finds a folder or a note in the tree, and nothing for a path that is not there', () => {
    expect(findNode(TREE, 'Notebooks/Projects/Shop')?.type).toBe('dir')
    expect(findNode(TREE, 'Notebooks/Projects/Shop/Stock rules.md')?.type).toBe('file')
    expect(findNode(TREE, 'Notebooks/Projects/Old shop')).toBeNull()
    expect(findNode(null, 'Notebooks')).toBeNull()
  })

  test('reads the Vault path from the address, decoding it', () => {
    expect(vaultLocation('/notes')).toEqual({ path: '', special: null })
    expect(vaultLocation('/notes/')).toEqual({ path: '', special: null })
    expect(vaultLocation('/notes/Notebooks/Projects/Shop%20site/Stock%20rules.md')).toEqual({
      path: 'Notebooks/Projects/Shop site/Stock rules.md',
      special: null,
    })
    expect(vaultLocation('/notes/trash')).toEqual({ path: '', special: 'trash' })
    expect(vaultLocation('/notes/n/abc123')).toEqual({ path: '', special: 'id' })
    expect(vaultLocation('/notes/Notebooks/100%')).toEqual({ path: 'Notebooks/100%', special: null })
  })
})

describe('the menu on a crumb', () => {
  test('lists the folders beside a folder, A to Z, with counts, and never an underscore folder', () => {
    const { parent, items } = siblingsOf(TREE, 'Notebooks/Projects')
    expect(parent).toBe('Notebooks')
    expect(items.map((i) => i.name)).toEqual(['Daily', 'Projects'])
    expect(items.find((i) => i.name === 'Projects')?.count).toBe(4)
  })

  test('lists the notes beside a note', () => {
    const { items } = siblingsOf(TREE, 'Notebooks/Projects/Shop/Stock rules.md')
    expect(items.map((i) => i.name)).toEqual(['Stock rules', 'Supplier terms'])
    expect(items.every((i) => !i.isDir)).toBe(true)
  })

  test('is empty for a path that is not in the tree', () => {
    expect(siblingsOf(TREE, 'Notebooks/Projects/Gone').items).toEqual([])
    expect(siblingsOf(null, 'Notebooks/Projects').items).toEqual([])
  })
})

describe('a path that is gone', () => {
  test('offers the deepest folder above it that still exists', () => {
    expect(nearestFolder(TREE, 'Notebooks/Projects/Shop/Old/Gone.md')).toBe('Notebooks/Projects/Shop')
    expect(nearestFolder(TREE, 'Notebooks/Projects/Gone')).toBe('Notebooks/Projects')
  })

  test('offers Vault home when nothing above it exists', () => {
    expect(nearestFolder(TREE, 'Notebooks/Lost/Gone.md')).toBe('')
    expect(nearestFolder(TREE, 'Lost/Gone.md')).toBe('')
  })

  test('says whether a path is in the tree', () => {
    expect(existsInTree(TREE, 'Notebooks/Projects/Shop')).toBe(true)
    expect(existsInTree(TREE, 'Notebooks/Projects/Old')).toBe(false)
    expect(existsInTree(null, 'Notebooks')).toBe(false)
  })
})

describe('system folders in the tree', () => {
  test('are listed and marked only when asked for, and a normal folder is not marked', () => {
    const off = treeRows(TREE, new Set())
    expect(off.some((r) => r.name === '_system')).toBe(false)
    const on = treeRows(TREE, new Set(), new Set(), 25, true)
    const sys = on.find((r) => r.name === '_system')
    expect(sys && sys.kind === 'folder' && sys.system).toBe(true)
    const daily = on.find((r) => r.name === 'Daily')
    expect(daily && daily.kind === 'folder' && daily.system).toBe(false)
  })

  test('their notes show when one is opened', () => {
    const rows = treeRows(TREE, new Set(['Notebooks/_system']), new Set(), 25, true)
    expect(rows.some((r) => r.kind === 'note' && r.name === 'rules')).toBe(true)
  })
})

describe('how far forward the history reaches', () => {
  test('a new page cuts off what lay ahead', () => {
    expect(furthestIndex(5, 'PUSH', 3)).toBe(3)
  })

  test('going back keeps it, so Forward stays available', () => {
    expect(furthestIndex(5, 'BACK', 3)).toBe(5)
    expect(furthestIndex(3, 'FORWARD', 4)).toBe(4)
  })

  test('replacing a page changes nothing ahead', () => {
    expect(furthestIndex(5, 'REPLACE', 3)).toBe(5)
  })
})
