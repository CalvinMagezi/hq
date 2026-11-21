import { describe, expect, test } from 'bun:test'
import { acceptCell, addRow, editable, setCell, setSlideBody, setSlideTitle } from './hqmdEdit'
import { readHqmd } from './hqmd'
import { computeSheet, readSheet } from './sheetCalc'

const NOTE = '---\nsuite_kind: sheet\n---\n# Stock\n\nSome prose here.\n\n```sheet name=Stock types="text,int,ugx,ugx"\n| item | qty | price | total |\n|------|-----|-------|-------|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Rice | 25 | 5600 | =B3*C3 |\n```\n\nAfter the sheet.\n'

describe('editing a cell', () => {
  test('changes that one line and leaves every other byte alone', () => {
    const next = setCell(NOTE, 0, 0, 1, '40', '50')!
    const a = NOTE.split('\n')
    const b = next.split('\n')
    expect(b).toHaveLength(a.length)
    expect(a.map((l, i) => (l === b[i] ? null : i)).filter((i) => i !== null)).toHaveLength(1)
    expect(next).toContain('| Sugar | 50 | 4200 | =B2*C2 |')
    expect(next.endsWith('After the sheet.\n')).toBe(true)
  })
  test('the formulas see the new value', () => {
    const next = setCell(NOTE, 0, 0, 1, '40', '50')!
    const sheet = readSheet(readHqmd(next).blocks[0])!
    expect(computeSheet(sheet)[0][3]).toEqual({ ok: true, value: 210000 })
  })
  test('is refused when the cell is not what the caller saw', () => {
    expect(setCell(NOTE, 0, 0, 1, '41', '50')).toBeNull()
    expect(setCell(NOTE, 0, 9, 1, '40', '50')).toBeNull()
    expect(setCell(NOTE, 1, 0, 1, '40', '50')).toBeNull()
  })
  test('keeps a pipe and a backslash in text from breaking the table', () => {
    const next = setCell(NOTE, 0, 0, 0, 'Sugar', 'a|b\\c')!
    expect(next).toContain('| a\\|b\\\\c |')
    expect(readSheet(readHqmd(next).blocks[0])!.rows[0][0]).toBe('a|b\\c')
  })
  test('numbers must be numbers, and money is stored plain', () => {
    expect(acceptCell('4,200', 'ugx')).toBe('4200')
    expect(acceptCell('UGX4200', 'ugx')).toBe('4200')
    expect(acceptCell('abc', 'int')).toBeNull()
    expect(acceptCell('=A1+1', 'int')).toBe('=A1+1')
    expect(acceptCell('2026-11-17', 'date')).toBe('2026-11-17')
    expect(acceptCell('soon', 'date')).toBeNull()
    expect(setCell(NOTE, 0, 0, 1, '40', 'lots')).toBeNull()
  })
  test('keeps CRLF notes CRLF', () => {
    const next = setCell(NOTE.replace(/\n/g, '\r\n'), 0, 0, 1, '40', '50')!
    expect(next.includes('\r\n')).toBe(true)
    expect(next.replace(/\r\n/g, '').includes('\n')).toBe(false)
  })
  test('adds an empty row at the end, and not to a sheet kept in a sidecar', () => {
    const next = addRow(NOTE, 0)!
    expect(readSheet(readHqmd(next).blocks[0])!.rows).toHaveLength(3)
    expect(addRow(NOTE.replace('types=', 'rows=9000 types='), 0)).toBeNull()
  })
})

const DECK = '---\nsuite_kind: slides\n---\n# Deck\n\n## One\n\nHello\n\n## Two\n<!-- layout: two-column -->\n\nWorld\n'

describe('editing slides', () => {
  test('renames a card, and only its heading line', () => {
    const next = setSlideTitle(DECK, 1, 'Two', 'Best sellers')!
    expect(next).toContain('## Best sellers')
    expect(next.replace('Best sellers', 'Two')).toBe(DECK)
    expect(setSlideTitle(DECK, 1, 'Zwei', 'x')).toBeNull()
    expect(setSlideTitle(DECK, 1, 'Two', '  ')).toBeNull()
  })
  test('rewrites a card body and keeps its layout comment', () => {
    const next = setSlideBody(DECK, 1, 'World', 'Sugar and rice')!
    expect(next).toContain('<!-- layout: two-column -->')
    const slides = readHqmd(next).slides
    expect(slides[1]).toMatchObject({ title: 'Two', body: 'Sugar and rice', layout: 'two-column' })
    expect(slides[0].body).toBe('Hello')
  })
  test('a body edit is refused when the card moved on', () => {
    expect(setSlideBody(DECK, 0, 'Goodbye', 'x')).toBeNull()
  })
})

describe('what may be edited', () => {
  test('a file kept view-only never is', () => {
    expect(editable('---\nsuite_fidelity: view\n---\n# x')).toBe(false)
    expect(editable('---\nsuite_fidelity: lossy\n---\n# x')).toBe(true)
    expect(editable('# plain')).toBe(true)
  })
})
