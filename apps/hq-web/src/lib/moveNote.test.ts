import { describe, expect, test } from 'bun:test'
import { normalizeTarget, placeOf } from './moveNote'

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
