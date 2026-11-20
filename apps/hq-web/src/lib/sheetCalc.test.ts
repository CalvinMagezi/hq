import { describe, expect, test } from 'bun:test'
import { typedBlocks } from './hqmd'
import { colName, computeSheet, formatValue, parseRange, readPlot, readRange, readSheet, tileValue, type Sheet } from './sheetCalc'

const block = (body: string, info = 'sheet name=Stock types="text,int,ugx,ugx"') => typedBlocks('```' + info + '\n' + body + '\n```\n')[0]
const SHEET = '| item | qty | price | total |\n|---|---|---|---|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Rice | 10 | 5000 | =B3*C3 |\n| Salt | 5 | 1000 | =B4*C4 |'
const sheet = (body = SHEET, info?: string): Sheet => readSheet(block(body, info))!
const value = (c: { ok: boolean }) => (c as { value: unknown }).value

describe('readSheet', () => {
  test('reads header, types, rows and the name', () => {
    const s = sheet()
    expect(s.name).toBe('Stock')
    expect(s.header).toEqual(['item', 'qty', 'price', 'total'])
    expect(s.types).toEqual(['text', 'int', 'ugx', 'ugx'])
    expect(s.rows).toHaveLength(3)
    expect(s.totalRows).toBeNull()
  })
  test('a missing type is text; a rows= attribute is the sidecar total', () => {
    const s = sheet('| a | b |\n|---|---|\n| 1 | 2 |', 'sheet types="int" rows=500')
    expect(s.types).toEqual(['int', 'text'])
    expect(s.totalRows).toBe(500)
  })
  test('a table with no delimiter row, or uneven rows, is not a sheet', () => {
    expect(readSheet(block('| a | b |\n| 1 | 2 |'))).toBeNull()
    expect(readSheet(block('| a | b |\n|---|---|\n| 1 |'))).toBeNull()
  })
  test('escaped pipes stay in the cell', () => {
    expect(sheet('| a |\n|---|\n| x \\| y |').rows[0][0]).toBe('x | y')
  })
  test('a block that is not a sheet gives null', () => {
    expect(readSheet(typedBlocks('```tile from=A1:A2 fn=sum\n```\n')[0])).toBeNull()
  })
})

describe('colName', () => {
  test('counts like a spreadsheet', () => {
    expect([0, 1, 25, 26, 27, 51, 52, 701, 702].map(colName)).toEqual(['A', 'B', 'Z', 'AA', 'AB', 'AZ', 'BA', 'ZZ', 'AAA'])
  })
})

describe('formatValue', () => {
  test('shows money, percentages, integers and booleans by type', () => {
    expect(formatValue(4200, 'ugx')).toBe('UGX 4,200')
    expect(formatValue(12.5, 'usd')).toBe('USD 12.50')
    expect(formatValue(0.12, 'pct')).toBe('12%')
    expect(formatValue(3.6, 'int')).toBe('4')
    expect(formatValue(1234.5, 'num')).toBe('1,234.50')
    expect(formatValue(true, 'bool')).toBe('Yes')
    expect(formatValue('x', 'text')).toBe('x')
  })
})

describe('computeSheet', () => {
  test('works out a product of two cells', () => {
    const c = computeSheet(sheet())
    expect(c[0][3]).toEqual({ ok: true, value: 168000 })
    expect(c[1][3]).toEqual({ ok: true, value: 50000 })
  })
  test('SUM, MIN, MAX, AVERAGE and COUNT over a range', () => {
    const s = sheet('| a | b |\n|---|---|\n| 2 | =SUM(A2:A4) |\n| 4 | =MIN(A2:A4) |\n| 6 | =MAX(A2:A4) |', 'sheet types="int,int"')
    expect(computeSheet(s).map((r) => value(r[1]))).toEqual([12, 2, 6])
    const more = sheet('| a | b |\n|---|---|\n| 2 | =AVERAGE(A2:A4) |\n| 4 | =COUNT(A2:A4) |\n| 6 | =A2+A3*A4 |', 'sheet types="int,num"')
    expect(computeSheet(more).map((r) => value(r[1]))).toEqual([4, 3, 26])
  })
  test('precedence, brackets and unary minus', () => {
    const s = sheet('| a |\n|---|\n| =(1+2)*3 |\n| =-2*3 |\n| =10/4 |\n| =2+3*4 |', 'sheet types="num"')
    expect(computeSheet(s).map((r) => value(r[0]))).toEqual([9, -6, 2.5, 14])
  })
  test('a function the mirror lacks, a division by zero and a bad reference are errors, never zero', () => {
    const s = sheet('| a |\n|---|\n| =NOPE(1) |\n| =1/0 |\n| =Z99 |\n| =SUM(A2:A3) |', 'sheet types="num"')
    const c = computeSheet(s)
    expect(c[0][0]).toEqual({ ok: false, why: 'formula' })
    expect(c[1][0]).toEqual({ ok: false, why: 'formula' })
    expect(c[2][0]).toEqual({ ok: false, why: 'formula' })
    // A range that holds an error cannot be summed.
    expect(c[3][0].ok).toBe(false)
  })
  test('a cell that reads itself does not loop', () => {
    expect(computeSheet(sheet('| a |\n|---|\n| =A2 |', 'sheet types="num"'))[0][0].ok).toBe(false)
  })
  test('plain cells: text stays text, numbers lose their commas, bool reads yes', () => {
    const s = sheet('| t | n | b | e |\n|---|---|---|---|\n| hi | 1,200 | yes | |\n| x | 12% | no | |', 'sheet types="text,num,bool,num"')
    const c = computeSheet(s)
    expect(c[0][0]).toEqual({ ok: true, value: 'hi' })
    expect(c[0][1]).toEqual({ ok: true, value: 1200 })
    expect(c[0][2]).toEqual({ ok: true, value: true })
    expect(c[1][2]).toEqual({ ok: true, value: false })
    expect(c[0][3]).toEqual({ ok: true, value: '' })
    expect(c[1][1]).toEqual({ ok: true, value: 12 })
  })
})

describe('parseRange', () => {
  test('reads a range, a single cell and a block-qualified range', () => {
    expect(parseRange('B2:B7')).toEqual({ c1: 1, r1: 2, c2: 1, r2: 7, sheet: null })
    expect(parseRange('C4')).toEqual({ c1: 2, r1: 4, c2: 2, r2: 4, sheet: null })
    expect(parseRange('[[#Stock]]!A1:B3')?.sheet).toBe('Stock')
  })
  test('another note, or nonsense, is null', () => {
    expect(parseRange('[[Other]]!A1:A3')).toBeNull()
    expect(parseRange('hello')).toBeNull()
  })
})

describe('readRange, tileValue and readPlot', () => {
  test('numbers and labels of a range', () => {
    expect(readRange([sheet()], 'B2:B4')).toEqual({ ok: true, numbers: [40, 10, 5], labels: ['Sugar', 'Rice', 'Salt'] })
  })
  test('a range of words has no numbers, a missing sheet and another note say why', () => {
    expect(readRange([sheet()], 'A2:A4')).toEqual({ ok: false, why: 'text' })
    expect(readRange([], 'B2:B4')).toEqual({ ok: false, why: 'no-sheet' })
    expect(readRange([sheet()], '[[Other]]!B2:B4')).toEqual({ ok: false, why: 'other-note' })
  })
  test('tile functions, and an unknown one is null', () => {
    const n = [3, 1, 2, 10]
    expect(tileValue('sum', n)).toBe(16)
    expect(tileValue('count', n)).toBe(4)
    expect(tileValue('avg', n)).toBe(4)
    expect(tileValue('min', n)).toBe(1)
    expect(tileValue('max', n)).toBe(10)
    expect(tileValue('median', n)).toBe(2.5)
    expect(tileValue('median', [5, 1, 3])).toBe(3)
    expect(tileValue('mode', n)).toBeNull()
    expect(tileValue('sum', [])).toBeNull()
  })
  test('a plot reads its labels and values by header name, formulas worked out', () => {
    expect(readPlot([sheet()], 'A1:D4', 'item', 'total')).toEqual({ ok: true, labels: ['Sugar', 'Rice', 'Salt'], values: [168000, 50000, 5000] })
    expect(readPlot([sheet()], 'A1:D4', 'item', 'nope')).toEqual({ ok: false, why: 'columns' })
  })
})
