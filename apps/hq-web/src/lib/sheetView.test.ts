import { describe, expect, test } from 'bun:test'
import { typedBlocks } from './hqmd'
import { computeSheet, readSheet } from './sheetCalc'
import { fillDownEdits, parseTsv, pasteEdits, recordFields, setCells, shiftFormula, toTsv, unworked, viewOrder, windowsFor } from './sheetView'

const NOTE = '```sheet name=S types="text,int,ugx"\n| item | qty | total |\n|---|---|---|\n| Sugar | 40 | =B2*10 |\n| Rice | 5 | =B3*10 |\n| apple | 12 | =NOPE(1) |\n| | 7 | 0 |\n```\n'
const sheet = () => readSheet(typedBlocks(NOTE)[0])!

describe('viewOrder', () => {
  test('sorts without touching the sheet, empty cells last either way', () => {
    const s = sheet()
    const c = computeSheet(s)
    expect(viewOrder(s, c, { sort: { col: 1, dir: 'asc' }, filter: null })).toEqual([1, 3, 2, 0])
    expect(viewOrder(s, c, { sort: { col: 1, dir: 'desc' }, filter: null })).toEqual([0, 2, 3, 1])
    expect(viewOrder(s, c, { sort: { col: 0, dir: 'asc' }, filter: null })).toEqual([2, 1, 0, 3])
    expect(viewOrder(s, c, { sort: { col: 0, dir: 'desc' }, filter: null })).toEqual([0, 1, 2, 3])
    expect(s.rows[0][0]).toBe('Sugar')
  })
  test('filters by what a cell shows, case blind', () => {
    const s = sheet()
    const c = computeSheet(s)
    expect(viewOrder(s, c, { sort: null, filter: { col: 0, text: 'AP' } })).toEqual([2])
    expect(viewOrder(s, c, { sort: null, filter: { col: 2, text: '400' } })).toEqual([0])
    expect(viewOrder(s, c, { sort: null, filter: { col: 0, text: '  ' } })).toEqual([0, 1, 2, 3])
  })
})

describe('clipboard', () => {
  test('a range copies as tab and newline text, formulas as typed', () => {
    expect(toTsv(sheet(), { r1: 0, c1: 0, r2: 1, c2: 2 })).toBe('Sugar\t40\t=B2*10\nRice\t5\t=B3*10')
    expect(toTsv(sheet(), { r1: 1, c1: 2, r2: 0, c2: 1 })).toBe('40\t=B2*10\n5\t=B3*10')
  })
  test('pasted text becomes a grid; one trailing newline is ignored', () => {
    expect(parseTsv('a\tb\r\nc\td\n')).toEqual([['a', 'b'], ['c', 'd']])
    expect(parseTsv('')).toEqual([])
  })
  test('a paste is checked against each column and the sheet edge', () => {
    const r = pasteEdits(sheet(), { row: 2, col: 0 }, [['kiwi', '9'], ['plum', 'many'], ['over', '1'], ['x|y', '1']])
    expect(r.edits.map((e) => [e.row, e.col, e.value])).toEqual([[2, 0, 'kiwi'], [2, 1, '9'], [3, 0, 'plum']])
    // "many" does not fit an int column; the two rows after the sheet's last row (two cells each) fall outside it.
    expect(r.skipped).toBe(5)
    expect(r.edits[0].was).toBe('apple')
  })
  test('setting the same text again is no edit', () => {
    expect(pasteEdits(sheet(), { row: 0, col: 0 }, [['Sugar']]).edits).toEqual([])
  })
})

describe('fill down', () => {
  test('relative references move, absolute ones stay, function names are left alone', () => {
    expect(shiftFormula('=B2*C2', 2)).toBe('=B4*C4')
    expect(shiftFormula('=$B$2*C$2+D2', 3)).toBe('=$B$2*C$2+D5')
    expect(shiftFormula('=LOG10(A2)', 1)).toBe('=LOG10(A3)')
    expect(shiftFormula('=SUM(A2:A4)', 1)).toBe('=SUM(A3:A5)')
  })
  test('the top row of a box is copied into the rows below (a cell that already holds it is no edit)', () => {
    const e = fillDownEdits(sheet(), { r1: 0, c1: 1, r2: 2, c2: 2 })
    expect(e.map((x) => [x.row, x.col, x.value])).toEqual([[1, 1, '40'], [2, 1, '40'], [2, 2, '=B4*10']])
  })
})

describe('setCells', () => {
  test('writes every edit into the note, and none when one is stale', () => {
    const done = setCells(NOTE, 0, [{ row: 1, col: 1, was: '5', value: '6' }, { row: 0, col: 0, was: 'Sugar', value: 'Salt' }])!
    expect(done).toContain('| Rice | 6 | =B3*10 |')
    expect(done).toContain('| Salt | 40 |')
    expect(setCells(NOTE, 0, [{ row: 1, col: 1, was: '5', value: '6' }, { row: 0, col: 0, was: 'Nope', value: 'x' }])).toBeNull()
  })
})

describe('record card and server windows', () => {
  test('one row as label, type and raw text', () => {
    expect(recordFields(sheet(), 1)).toEqual([
      { col: 0, header: 'item', type: 'text', raw: 'Rice' },
      { col: 1, header: 'qty', type: 'int', raw: '5' },
      { col: 2, header: 'total', type: 'ugx', raw: '=B3*10' },
    ])
  })
  test('cells the mirror cannot work out are listed, and grouped into the server windows that hold them', () => {
    const s = sheet()
    expect(unworked(s, computeSheet(s))).toEqual([[2, 2]])
    expect(windowsFor([[2, 2], [3, 9], [40, 12]])).toEqual([{ row: 0, col: 0 }, { row: 30, col: 10 }])
    expect(windowsFor(Array.from({ length: 20 }, (_, i) => [i * 40, 0] as [number, number]))).toHaveLength(6)
  })
})
