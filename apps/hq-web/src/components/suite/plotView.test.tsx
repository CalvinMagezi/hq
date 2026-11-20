import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { PlotChart } from './PlotView'

const draw = (kind: string, labels: string[], series: { name: string; values: number[] }[]) => renderToStaticMarkup(createElement(PlotChart, { kind, title: 'T', labels, series, format: 'num' }))
const TWO = [
  { name: 'North', values: [1, 4, 2] },
  { name: 'South', values: [3, 2, 5] },
]
const L = ['Jan', 'Feb', 'Mar']

describe('the plot kinds', () => {
  test('an area fills under each line and keeps the dash on the second', () => {
    const html = draw('area', L, TWO)
    expect(html.match(/<polygon/g)).toHaveLength(2)
    expect(html.match(/<polyline/g)).toHaveLength(2)
    expect(html.match(/stroke-dasharray="6 4"/g)?.length).toBeGreaterThanOrEqual(1)
  })
  test('a stacked chart has a column per label and a segment per series, with a hatch on the later ones', () => {
    const html = draw('stacked', L, TWO)
    expect(html.match(/class="su-seg"/g)).toHaveLength(6)
    expect(html).toContain('url(#')
    // The axis holds the stack (4 + 2 and 5 + 3 at most), not the biggest single value.
    expect(html).toContain('>8<')
  })
  test('a scatter chart draws a marker per point on a numeric x axis, one shape per series', () => {
    const html = draw('scatter', ['1', '2.5', '4'], [{ name: 'a', values: [1, 3, 2] }, { name: 'b', values: [2, Number.NaN, 3] }])
    expect(html.match(/class="su-dot"/g)).toHaveLength(5)
    expect(html).not.toContain('<polyline')
    expect(html).not.toContain('NaN')
  })
  test('a scatter chart whose labels are not numbers, or a kind that is not known, says it cannot be drawn', () => {
    for (const html of [draw('scatter', L, TWO), draw('radar', L, TWO)]) {
      expect(html).toContain('could not be drawn')
      expect(html).not.toContain('<svg class="su-svg"')
    }
  })
  test('bar, line and pie are as they were', () => {
    expect(draw('bar', L, TWO).match(/<rect/g)?.length).toBeGreaterThanOrEqual(12)
    expect(draw('line', L, TWO).match(/<polyline/g)).toHaveLength(2)
    expect(draw('pie', L, [TWO[0]])).toContain('su-slice')
  })
})
