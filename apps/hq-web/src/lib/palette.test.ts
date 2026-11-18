import { describe, expect, test } from 'bun:test'
import { doActions, matchActions, matchFolders, previewAllowed, previewLines } from './palette'
import type { NoteTreeNode } from './vaultApi'

const dir = (path: string, children: NoteTreeNode[] = []): NoteTreeNode => ({ name: path.split('/').pop() ?? path, path, type: 'dir', children })
const TREE = dir('Notebooks', [
  dir('Notebooks/_system', [dir('Notebooks/_system/shop orders')]),
  dir('Notebooks/Projects', [dir('Notebooks/Projects/Shop', [dir('Notebooks/Projects/Shop/Orders')]), dir('Notebooks/Projects/Website')]),
  dir('Notebooks/Orders archive'),
])

describe('the actions the palette can do', () => {
  test('the work pane is offered only in a chat, and Lite has no chat actions', () => {
    expect(doActions({ inChat: false, lite: false }).map((a) => a.id)).not.toContain('pane')
    expect(doActions({ inChat: true, lite: false }).map((a) => a.id)).toContain('pane')
    const lite = doActions({ inChat: true, lite: true }).map((a) => a.id)
    expect(lite).toContain('new-note')
    expect(lite).not.toContain('new-chat')
    expect(lite).not.toContain('go-chat')
  })
  test('actions with a chord say it, and every chord named is one the app handles', () => {
    const keys = Object.fromEntries(doActions({ inChat: true, lite: false }).map((a) => [a.id, a.keys]))
    expect(keys['new-note']).toBe('Ctrl Alt N')
    expect(keys['new-chat']).toBe('Ctrl Shift O')
    expect(keys.pane).toBe('Ctrl .')
  })
  test('matches the words typed against the names', () => {
    const names: Record<string, string> = { 'new-note': 'New note', 'new-chat': 'New chat', 'go-vault': 'Go to the Vault' }
    const all = doActions({ inChat: false, lite: false })
    expect(matchActions(all, 'new', (a) => names[a.id] ?? '').map((a) => a.id)).toEqual(['new-note', 'new-chat'])
    expect(matchActions(all, 'vault', (a) => names[a.id] ?? '').map((a) => a.id)).toEqual(['go-vault'])
  })
})

describe('folders that match what was typed', () => {
  test('part of a path lists the folder, shortest first, never a system folder', () => {
    const hits = matchFolders(TREE, 'shop orders')
    expect(hits.map((h) => h.path)).toEqual(['Notebooks/Projects/Shop/Orders'])
    expect(hits[0].trail).toBe('Projects › Shop › Orders')
    expect(matchFolders(TREE, 'orders').map((h) => h.path)).toEqual(['Notebooks/Orders archive', 'Notebooks/Projects/Shop/Orders'])
  })
  test('nothing typed, or no tree yet, lists nothing', () => {
    expect(matchFolders(TREE, '  ')).toEqual([])
    expect(matchFolders(undefined, 'shop')).toEqual([])
  })
})

describe('the preview', () => {
  test('shows the first lines without frontmatter or blank lines', () => {
    const note = '---\ntitle: x\npinned: true\n---\n\n# Stock rules\n\nReorder at ten.\n\n\nCheck on Friday.\n'
    expect(previewLines(note, 2)).toEqual(['# Stock rules', 'Reorder at ten.'])
  })
  test('is not fetched when the browser asked to save data', () => {
    expect(previewAllowed({ saveData: true })).toBe(false)
    expect(previewAllowed({ saveData: false })).toBe(true)
    expect(previewAllowed(undefined)).toBe(true)
  })
})
