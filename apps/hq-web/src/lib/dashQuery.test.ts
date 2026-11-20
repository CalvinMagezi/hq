import { describe, expect, test } from 'bun:test'
import { typedBlocks } from './hqmd'
import { computeSheet, readSheet } from './sheetCalc'
import { filterColumns, matchingRows, parseWhere, plotData, tileNumbers } from './dashQuery'

const NOTE = '```sheet name=Sales types="text,text,int"\n| month | region | amount |\n|---|---|---|\n| Jan | North | 10 |\n| Jan | South | 20 |\n| Feb | North | 30 |\n| Feb | South | 5 |\n```\n'
const sheets = () => [readSheet(typedBlocks(NOTE)[0])!]

describe('where', () => {
  test('reads up to four conditions, numbers and words', () => {
    expect(parseWhere('amount>15 and region=North')).toEqual([{ column: 'amount', op: '>', value: 15 }, { column: 'region', op: '=', value: 'North' }])
    expect(parseWhere('a=1 and b=2 and c=3 and d=4 and e=5')).toBeNull()
    expect(parseWhere('amount>')).toBeNull()
    expect(parseWhere('region="North"')?.[0].value).toBe('North')
  })
  test('rows that match, text blind to case, != holding across kinds', () => {
    const [s] = sheets()
    const c = computeSheet(s)
    expect(matchingRows(s, c, 'region=north', [])).toEqual([0, 2])
    expect(matchingRows(s, c, 'amount>=20', [])).toEqual([1, 2])
    expect(matchingRows(s, c, 'amount!=North', [])).toEqual([0, 1, 2, 3])
    expect(matchingRows(s, c, 'amount>North', [])).toEqual([])
    expect(matchingRows(s, c, 'nope=1', [])).toBeNull()
  })
  test('a page filter narrows further and ignores a column the sheet lacks', () => {
    const [s] = sheets()
    const c = computeSheet(s)
    expect(matchingRows(s, c, 'amount>5', [{ column: 'month', value: 'Feb' }])).toEqual([2])
    expect(matchingRows(s, c, undefined, [{ column: 'zzz', value: 'x' }])).toEqual([0, 1, 2, 3])
  })
})

describe('dropdown choices', () => {
  test('text columns with a few distinct values, sorted', () => {
    expect(filterColumns(sheets()[0])).toEqual([{ column: 'month', values: ['Feb', 'Jan'] }, { column: 'region', values: ['North', 'South'] }])
  })
})

describe('tiles', () => {
  test('without a where every number in the range; with one, the last column of the matching rows', () => {
    expect(tileNumbers(sheets(), 'C2:C5', undefined, [])).toEqual({ ok: true, numbers: [10, 20, 30, 5] })
    expect(tileNumbers(sheets(), 'A1:C5', 'region=North', [])).toEqual({ ok: true, numbers: [10, 30] })
  })
  test('the page filter applies on top and an empty result says there are no numbers', () => {
    expect(tileNumbers(sheets(), 'C2:C5', undefined, [{ column: 'region', value: 'South' }])).toEqual({ ok: true, numbers: [20, 5] })
    expect(tileNumbers(sheets(), 'C2:C5', 'amount>99', [])).toEqual({ ok: false, why: 'text' })
    expect(tileNumbers(sheets(), '[[Other]]!C2:C5', undefined, [])).toEqual({ ok: false, why: 'other-note' })
  })
})

describe('plots', () => {
  test('one series, and a group makes one per value with a missing label as zero', () => {
    const a = { from: 'A1:C5', x: 'month', y: 'amount' }
    expect(plotData(sheets(), a, [])).toEqual({ ok: true, labels: ['Jan', 'Feb'], series: [{ name: 'amount', values: [30, 35] }] })
    expect(plotData(sheets(), { ...a, group: 'region' }, [])).toEqual({ ok: true, labels: ['Jan', 'Feb'], series: [{ name: 'North', values: [10, 30] }, { name: 'South', values: [20, 5] }] })
    const w = plotData(sheets(), { ...a, group: 'region', where: 'amount>6' }, [])
    expect(w.ok && w.series).toEqual([{ name: 'North', values: [10, 30] }, { name: 'South', values: [20, 0] }])
  })
  test('unknown columns and too many groups say why', () => {
    expect(plotData(sheets(), { from: 'A1:C5', x: 'month', y: 'nope' }, [])).toEqual({ ok: false, why: 'columns' })
    const many = '```sheet types="text,text,int"\n| m | g | v |\n|---|---|---|\n' + Array.from({ length: 7 }, (_, i) => `| a | g${i} | 1 |`).join('\n') + '\n```\n'
    expect(plotData([readSheet(typedBlocks(many)[0])!], { from: 'A1:C9', x: 'm', y: 'v', group: 'g' }, [])).toEqual({ ok: false, why: 'groups' })
  })
  test('a page filter narrows a plot', () => {
    expect(plotData(sheets(), { from: 'A1:C5', x: 'month', y: 'amount' }, [{ column: 'region', value: 'North' }])).toEqual({ ok: true, labels: ['Jan', 'Feb'], series: [{ name: 'amount', values: [10, 30] }] })
  })
})

describe('scatter', () => {
  const XY = '```sheet name=Pairs types="num,num,text"\n| x | y | team |\n|---|---|---|\n| 1 | 10 | a |\n| 1 | 12 | b |\n| 2.5 | 7 | a |\n| 4 | 9 | b |\n```\n'
  const pairs = () => [readSheet(typedBlocks(XY)[0])!]
  test('keeps every row as a point, even two with the same x', () => {
    const r = plotData(pairs(), { from: 'A1:C5', x: 'x', y: 'y', kind: 'scatter' }, [])
    expect(r.ok).toBe(true)
    if (r.ok) {
      expect(r.labels).toEqual(['1', '1', '2.5', '4'])
      expect(r.series[0].values).toEqual([10, 12, 7, 9])
    }
  })
  test('a group gets NaN where it has no point, so the point is not drawn', () => {
    const r = plotData(pairs(), { from: 'A1:C5', x: 'x', y: 'y', group: 'team', kind: 'scatter' }, [])
    expect(r.ok).toBe(true)
    if (r.ok) {
      expect(r.series.map((s) => s.name)).toEqual(['a', 'b'])
      expect(r.series[0].values.map(Number.isNaN)).toEqual([false, true, false, true])
      expect(r.series[1].values.map(Number.isNaN)).toEqual([true, false, true, false])
    }
  })
  test('an x that is not a number cannot be drawn, and says the range holds no numbers', () => {
    const r = plotData(sheets(), { from: 'A1:C5', x: 'month', y: 'amount', kind: 'scatter' }, [])
    expect(r).toEqual({ ok: false, why: 'text' })
  })
  test('the same range as a bar still adds the rows of one label together', () => {
    const r = plotData(pairs(), { from: 'A1:C5', x: 'x', y: 'y' }, [])
    expect(r.ok).toBe(true)
    if (r.ok) expect(r.labels).toEqual(['1', '2.5', '4'])
  })
})
