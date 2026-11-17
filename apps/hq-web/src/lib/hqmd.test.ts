import { describe, expect, test } from 'bun:test'
import { hasViewer, parseHeader, readHqmd, slidesOf, splitFrontmatter, typedBlocks } from './hqmd'
import { colName, computeSheet, formatValue, parseRange, readRange, readSheet, tileValue } from './sheetCalc'

const SHEET = '```sheet name=Stock types="text,int,ugx,ugx" id=stock\n| item | qty | price | total |\n|------|-----|-------|-------|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Rice | 25 | 5600 | =B3*C3 |\n| Total | | | =SUM(D2:D3) |\n```\n'
const NOTE = `---\nsuite_kind: dashboard\nsuite_fidelity: native\n---\n# Weekly stock\n\n${SHEET}\n\`\`\`tile from=D2:D3 fn=sum label="Stock value" format=ugx\n\`\`\`\n\n\`\`\`plot type=bar from=D2:D3 x=item y=total\n\`\`\`\n`

describe('reading hqmd', () => {
  test('frontmatter keys are flat, and a note without any has none', () => {
    expect(splitFrontmatter('---\nsuite_kind: sheet\nsuite_fidelity: "safe"\n---\nBody').meta).toEqual({ suite_kind: 'sheet', suite_fidelity: 'safe' })
    expect(splitFrontmatter('# Plain').meta).toEqual({})
  })
  test('a header gives its type and attributes, quoted or bare', () => {
    expect(parseHeader('sheet name=Stock types="text,int" id=stock')).toEqual({ type: 'sheet', attrs: { name: 'Stock', types: 'text,int', id: 'stock' } })
    expect(parseHeader('python')).toBeNull()
    expect(parseHeader('tile label="a \\"b\\""')?.attrs.label).toBe('a "b"')
  })
  test('finds typed blocks and leaves other code alone', () => {
    const note = readHqmd(NOTE)
    expect(note.kind).toBe('dashboard')
    expect(note.title).toBe('Weekly stock')
    expect(note.blocks.map((b) => b.type)).toEqual(['sheet', 'tile', 'plot'])
    expect(typedBlocks('```python\nprint(1)\n```\n')).toEqual([])
    expect(typedBlocks('  ```sheet\n| a |\n```')).toEqual([])
  })
  test('slides are the level 2 headings, with an optional layout', () => {
    const slides = slidesOf('# Deck\n\n## One\n\nHello\n\n## Two\n<!-- layout: two-column -->\nWorld\n')
    expect(slides).toEqual([{ title: 'One', body: 'Hello', layout: null }, { title: 'Two', body: 'World', layout: 'two-column' }])
  })
  test('only sheets, slides and dashboards get a viewer', () => {
    expect(hasViewer(readHqmd(NOTE))).toBe(true)
    expect(hasViewer(readHqmd('---\nsuite_kind: doc\n---\n# D'))).toBe(false)
    expect(hasViewer(readHqmd('# Plain note'))).toBe(false)
  })
})

describe('a sheet', () => {
  const sheet = readSheet(typedBlocks(SHEET)[0])!
  test('reads its table and types', () => {
    expect(sheet.header).toEqual(['item', 'qty', 'price', 'total'])
    expect(sheet.types).toEqual(['text', 'int', 'ugx', 'ugx'])
    expect(sheet.rows).toHaveLength(3)
    expect(readSheet({ type: 'sheet', attrs: {}, body: '| a | b |\n| c |' })).toBeNull()
  })
  test('works out formulas, with cell references and a sum over a range', () => {
    const cells = computeSheet(sheet)
    expect(cells[0][3]).toEqual({ ok: true, value: 168000 })
    expect(cells[1][3]).toEqual({ ok: true, value: 140000 })
    expect(cells[2][3]).toEqual({ ok: true, value: 308000 })
  })
  test('a formula it cannot read, or that loops, says so and is not a zero', () => {
    const bad = readSheet({ type: 'sheet', attrs: { types: 'int,int' }, body: '| a | b |\n|---|---|\n| =FOO(1) | =B2 |\n| =A3 | 1/0 |' })!
    expect(computeSheet(bad)[0][0]).toEqual({ ok: false, why: 'formula' })
    expect(computeSheet(bad)[1][0]).toEqual({ ok: false, why: 'formula' })
  })
  test('shows money by its column type, never as a stray decimal', () => {
    expect(formatValue(4200, 'ugx')).toBe('UGX 4,200')
    expect(formatValue(0.12, 'pct')).toBe('12%')
    expect(formatValue(1234.5, 'num')).toBe('1,234.50')
    expect(colName(0)).toBe('A')
    expect(colName(27)).toBe('AB')
  })
})

describe('tiles and plots read ranges', () => {
  const sheets = [readSheet(typedBlocks(SHEET)[0])!]
  test('a range in this note gives its numbers and labels', () => {
    const r = readRange(sheets, 'D2:D3')
    expect(r).toEqual({ ok: true, numbers: [168000, 140000], labels: ['Sugar', 'Rice'] })
  })
  test('a range in another note, or in text, is not guessed at', () => {
    expect(readRange(sheets, '[[Other]]!A1:A2')).toEqual({ ok: false, why: 'other-note' })
    expect(readRange(sheets, 'A2:A3')).toEqual({ ok: false, why: 'text' })
    expect(parseRange('B2:C9')).toMatchObject({ c1: 1, r1: 2, c2: 2, r2: 9 })
  })
  test('the tile functions', () => {
    const n = [4, 1, 3, 2]
    expect([tileValue('sum', n), tileValue('count', n), tileValue('avg', n), tileValue('min', n), tileValue('max', n), tileValue('median', n)]).toEqual([10, 4, 2.5, 1, 4, 2.5])
    expect(tileValue('sum', [])).toBeNull()
    expect(tileValue('mode', n)).toBeNull()
  })
})

describe('a plot reads its columns by name', () => {
  const sheets = [readSheet(typedBlocks(SHEET)[0])!]
  test('header row first, then a label column and a value column', async () => {
    const { readPlot } = await import('./sheetCalc')
    expect(readPlot(sheets, 'A1:D3', 'item', 'total')).toEqual({ ok: true, labels: ['Sugar', 'Rice'], values: [168000, 140000] })
    expect(readPlot(sheets, 'A1:D3', 'item', 'missing')).toEqual({ ok: false, why: 'columns' })
    expect(readPlot(sheets, '[[Other]]!A1:D3', 'item', 'total')).toEqual({ ok: false, why: 'other-note' })
  })
})
