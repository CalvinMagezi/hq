import { describe, expect, test } from 'bun:test'
import { dropTarget, normalizeTarget, placeOf } from './moveNote'

const here = 'Notebooks/Inbox/Idea.md'

describe('where a note can be moved to', () => {
  test('adds .md, tidies slashes and keeps it under Notebooks', () => {
    expect(normalizeTarget('Notebooks//Projects/Plan', here)).toEqual({ path: 'Notebooks/Projects/Plan.md', problem: null })
    expect(normalizeTarget('  Notebooks/Plan.MD ', here)).toEqual({ path: 'Notebooks/Plan.MD', problem: null })
    expect(normalizeTarget('Notebooks\\Shop\\Orders', here).path).toBe('Notebooks/Shop/Orders.md')
  })
  test('says why when it cannot be one', () => {
    expect(normalizeTarget('Elsewhere/Plan.md', here).problem).toBe('move_bad_start')
    expect(normalizeTarget('Notebooks/', here).problem).toBe('move_bad_name')
    expect(normalizeTarget('Notebooks/.md', here).problem).toBe('move_bad_name')
    expect(normalizeTarget('Notebooks/a?b.md', here).problem).toBe('move_bad_chars')
    expect(normalizeTarget('Notebooks/../_system/x.md', here).problem).toBe('move_bad_chars')
    expect(normalizeTarget(here, here).problem).toBe('move_same')
  })
  test('names a place for a toast', () => {
    expect(placeOf('Notebooks/Projects/Plan.md')).toBe('Projects/Plan')
  })
})

describe('moving a folder', () => {
  test('a folder path has no .md, and cannot go inside itself', () => {
    expect(normalizeTarget('Notebooks/Archive/Old/', 'Notebooks/Inbox/Old', 'folder')).toEqual({ path: 'Notebooks/Archive/Old', problem: null })
    expect(normalizeTarget('Notebooks/Inbox/Old/Deep/Old', 'Notebooks/Inbox/Old', 'folder').problem).toBe('move_into_itself')
    expect(normalizeTarget('Notebooks', 'Notebooks/Inbox/Old', 'folder').problem).toBe('move_bad_start')
    expect(normalizeTarget('Notebooks/Inbox/Old', 'Notebooks/Inbox/Old', 'folder').problem).toBe('move_same')
  })
})

describe('dropping on a folder', () => {
  test('puts a note or folder inside it, keeping its name', () => {
    expect(dropTarget({ kind: 'note', path: 'Notebooks/Inbox/Idea.md' }, 'Notebooks/Projects')).toEqual({ from: 'Notebooks/Inbox/Idea.md', to: 'Notebooks/Projects/Idea.md', kind: 'note' })
    expect(dropTarget({ kind: 'folder', path: 'Notebooks/Inbox/Old' }, 'Notebooks')).toEqual({ from: 'Notebooks/Inbox/Old', to: 'Notebooks/Old', kind: 'folder' })
  })
  test('does nothing for its own folder, itself, its insides or a system folder', () => {
    expect(dropTarget({ kind: 'note', path: 'Notebooks/Inbox/Idea.md' }, 'Notebooks/Inbox')).toBeNull()
    expect(dropTarget({ kind: 'folder', path: 'Notebooks/Inbox/Old' }, 'Notebooks/Inbox/Old')).toBeNull()
    expect(dropTarget({ kind: 'folder', path: 'Notebooks/Inbox/Old' }, 'Notebooks/Inbox/Old/Deep')).toBeNull()
    expect(dropTarget({ kind: 'note', path: 'Notebooks/Inbox/Idea.md' }, 'Notebooks/_system')).toBeNull()
    expect(dropTarget({ kind: 'note', path: 'Notebooks/_system/x.md' }, 'Notebooks/Projects')).toBeNull()
    expect(dropTarget({ kind: 'folder', path: 'Notebooks' }, 'Notebooks/Projects')).toBeNull()
  })
})
