import { expect, test } from 'bun:test'
import { MARKER_SHAPES, MAX_POINTS, MAX_SERIES, MAX_SLICES, areaPoints, markerPath, niceStep, parseChart, parseMermaidPie, renderChartHtml, stackBands, xScale, yScale, yValuesOf, type ChartSpec } from './chart'

const payload = (over: Record<string, unknown> = {}) =>
  JSON.stringify({
    type: 'line',
    title: 'Sales',
    xLabel: 'Month',
    yLabel: 'Units',
    labels: ['Jan', 'Feb', 'Mar'],
    series: [
      { name: 'North', values: [1, 4, 2] },
      { name: 'South', values: [3, 2, 5] },
    ],
    ...over,
  })

const render = (over: Record<string, unknown> = {}) => {
  const parsed = parseChart(payload(over))
  if (!parsed.ok) throw new Error(parsed.reason)
  return renderChartHtml(parsed.spec)
}

test('line chart renders title, axis labels, legend and one polyline per series', () => {
  const html = render()
  expect(html).toContain('Sales')
  expect(html).toContain('>Month<')
  expect(html).toContain('>Units<')
  expect(html.match(/<polyline/g)).toHaveLength(2)
  expect(html).toContain('North')
  expect(html).toContain('South')
})

test('bar chart renders one rect per value', () => {
  const html = render({ type: 'bar' })
  expect(html.match(/<rect/g)).toHaveLength(6)
})

test('pie and donut render a wedge per slice and a legend with percentages', () => {
  const pie = render({ type: 'pie', labels: ['A', 'B'], series: [{ name: 's', values: [1, 3] }] })
  expect(pie.match(/<path/g)).toHaveLength(2)
  expect(pie).toContain('A: 1 (25%)')
  expect(pie).toContain('B: 3 (75%)')
  const donut = render({ type: 'donut', labels: ['A'], series: [{ name: 's', values: [5] }] })
  expect(donut.match(/<path/g)).toHaveLength(1)
  expect(donut).toContain('(100%)')
})

test('output scales with its container: viewBox and fluid width, no fixed pixel size', () => {
  const html = render()
  expect(html).toContain('viewBox=')
  expect(html).toContain('width:100%')
  expect(html).not.toMatch(/<svg[^>]* width="\d+"/)
})

test('negative values keep a zero baseline inside the scale', () => {
  const s = yScale([-5, 3])
  expect(s.min).toBeLessThanOrEqual(-5)
  expect(s.max).toBeGreaterThanOrEqual(3)
  expect(s.ticks).toContain(0)
  expect(render({ type: 'bar', series: [{ name: 'n', values: [-2, 0, 4] }] })).toContain('<rect')
})

test('an all-zero series still produces a valid scale', () => {
  const s = yScale([0, 0])
  expect(s.max).toBeGreaterThan(s.min)
  expect(niceStep(1, 4)).toBe(0.5)
})

test('user text is escaped', () => {
  const html = render({ title: '<script>alert(1)</script>', labels: ['<img onerror=x>', 'b', 'c'] })
  expect(html).not.toContain('<script>')
  expect(html).not.toContain('<img')
  expect(html).toContain('&lt;script&gt;')
})

test('invalid payloads report a reason instead of throwing', () => {
  const cases: [string, string][] = [
    ['', 'not valid JSON'],
    ['{oops', 'not valid JSON'],
    ['[1,2]', 'JSON object'],
    [payload({ type: 'radar' }), 'The type must be'],
    [payload({ labels: [] }), 'at least one label'],
    [payload({ labels: 'abc' }), 'at least one label'],
    [payload({ series: [] }), 'at least one series'],
    [payload({ series: [{ name: 'x', values: [1, 2] }] }), 'needs 3 values'],
    [payload({ series: [{ name: 'x', values: [1, 'two', 3] }] }), 'not a number'],
    [payload({ series: [{ name: 'x', values: [1, null, 3] }] }), 'not a number'],
    [payload({ series: [{ name: 'x' }] }), 'needs a list of values'],
    [payload({ type: 'pie', series: [{ name: 'x', values: [1, -1, 3] }] }), 'negative value'],
    [payload({ type: 'donut', series: [{ name: 'x', values: [0, 0, 0] }] }), 'add up to more than zero'],
  ]
  for (const [src, reason] of cases) {
    const r = parseChart(src)
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.reason).toContain(reason)
  }
})

test('size limits are enforced', () => {
  const many = Array.from({ length: MAX_POINTS + 1 }, (_, i) => String(i))
  expect(parseChart(payload({ labels: many, series: [{ name: 'a', values: many.map(() => 1) }] })).ok).toBe(false)
  const slices = Array.from({ length: MAX_SLICES + 1 }, (_, i) => String(i))
  expect(parseChart(payload({ type: 'pie', labels: slices, series: [{ name: 'a', values: slices.map(() => 1) }] })).ok).toBe(false)
  const series = Array.from({ length: MAX_SERIES + 1 }, () => ({ name: 's', values: [1, 2, 3] }))
  expect(parseChart(payload({ series })).ok).toBe(false)
})

test('a chart without title or axis labels still renders', () => {
  const parsed = parseChart(JSON.stringify({ type: 'bar', labels: ['a'], series: [{ name: 's', values: [2] }] }))
  expect(parsed.ok).toBe(true)
  const spec = (parsed as { ok: true; spec: ChartSpec }).spec
  expect(renderChartHtml(spec)).toContain('<svg')
  expect(renderChartHtml(spec)).not.toContain('<figcaption')
})

const MERMAID_PIE = 'pie showData\n    title This week outflows (UGX)\n    "School fees" : 2775000\n    "Loans" : 1502387\n'

test('converts a Mermaid pie into a pie chart', () => {
  const result = parseMermaidPie(MERMAID_PIE)
  expect(result.ok).toBe(true)
  if (!result.ok) return
  expect(result.spec.type).toBe('pie')
  expect(result.spec.labels).toEqual(['School fees', 'Loans'])
  expect(result.spec.series[0].values).toEqual([2775000, 1502387])
  expect(result.spec.title).toBe('This week outflows (UGX)')
})

test('leaves other Mermaid diagrams and unknown syntax alone', () => {
  expect(parseMermaidPie('graph TD\n A-->B').ok).toBe(false)
  expect(parseMermaidPie('pie\n "a" : 1\n click a callback').ok).toBe(false)
  expect(parseMermaidPie('pie\n').ok).toBe(false)
})

test('colours come from the brand palette and nothing else', () => {
  const html = render({ type: 'line', series: [{ name: 'a', values: [1, 2, 3] }, { name: 'b', values: [3, 2, 1] }] })
  expect(html).toContain('var(--brand-blue)')
  expect(html).toContain('var(--brand-sky)')
  expect(html).not.toContain('--accent-')
})

test('a second line is dashed and the legend says which is which', () => {
  const html = render({ type: 'line', series: [{ name: 'a', values: [1, 2, 3] }, { name: 'b', values: [3, 2, 1] }] })
  expect(html.match(/stroke-dasharray/g)).toHaveLength(1)
  expect(html).toContain('a · solid line')
  expect(html).toContain('b · dashed line')
})

test('failure reasons are plain sentences with no field names', () => {
  const r = parseChart(payload({ series: [{ name: 'x', values: [1, 2] }] }))
  expect(r.ok).toBe(false)
  if (!r.ok) expect(r.reason).toBe('Series 1 needs 3 values, one for each label.')
})

test('an area chart fills under each line and keeps the line, dashed after the first', () => {
  const html = render({ type: 'area' })
  expect(html.match(/<polygon/g)).toHaveLength(2)
  expect(html.match(/<polyline/g)).toHaveLength(2)
  expect(html.match(/stroke-dasharray/g)).toHaveLength(1)
  expect(html).toContain('North · solid line')
  expect(html).toContain('South · dashed line')
})

test('an area polygon closes along the baseline', () => {
  expect(areaPoints([{ x: 10, y: 5 }, { x: 20, y: 8 }], 30)).toBe('10,5 20,8 20,30 10,30')
  expect(areaPoints([], 30)).toBe('')
})

test('a stacked chart has one column per label and series, each on top of the one before', () => {
  const html = render({ type: 'stacked' })
  expect(html.match(/<rect/g)).toHaveLength(6)
  expect(html).toContain('var(--brand-blue)')
  expect(html).toContain('var(--brand-sky)')
})

test('stacks add positives upwards and negatives downwards, apart from each other', () => {
  const bands = stackBands([{ values: [2, -1, 3] }, { values: [3, -2, -1] }])
  expect(bands[0].map((b) => [b.lo, b.hi])).toEqual([[0, 2], [-1, 0], [0, 3]])
  expect(bands[1].map((b) => [b.lo, b.hi])).toEqual([[2, 5], [-3, -1], [-1, 0]])
  // The y axis has to hold the whole stack, not the biggest single value.
  expect(Math.max(...yValuesOf('stacked', [{ values: [2, -1, 3] }, { values: [3, -2, -1] }]))).toBe(5)
  expect(Math.max(...yValuesOf('bar', [{ values: [2, -1, 3] }, { values: [3, -2, -1] }]))).toBe(3)
  expect(yScale(yValuesOf('stacked', [{ values: [4] }, { values: [4] }])).max).toBeGreaterThanOrEqual(8)
})

test('a scatter chart draws one marker per point, the x taken from the label, with its own shape per series', () => {
  const html = render({ type: 'scatter', labels: ['1', '2.5', '4'], series: [{ name: 'a', values: [1, 3, 2] }, { name: 'b', values: [2, 1, 3] }] })
  expect(html.match(/<path/g)).toHaveLength(6)
  expect(html).not.toContain('<polyline')
  // The x axis is numbers, not the labels as words.
  expect(html).toContain('>1<')
  expect(html).toContain('>4<')
  expect(markerPath(0, 10, 10, 3)).not.toBe(markerPath(1, 10, 10, 3))
  expect(MARKER_SHAPES.length).toBeGreaterThanOrEqual(4)
})

test('a scatter chart whose labels are not numbers says so', () => {
  const r = parseChart(payload({ type: 'scatter' }))
  expect(r.ok).toBe(false)
  if (!r.ok) expect(r.reason).toContain('number')
  expect(parseChart(payload({ type: 'scatter', labels: [1, 2, 3] })).ok).toBe(true)
  expect(parseChart(payload({ type: 'scatter', labels: ['1', '', '3'] })).ok).toBe(false)
})

test('the x axis of a scatter chart rounds outwards and does not have to reach zero', () => {
  const x = xScale([103, 118, 141])
  expect(x.min).toBeLessThanOrEqual(103)
  expect(x.max).toBeGreaterThanOrEqual(141)
  expect(x.min).toBeGreaterThan(0)
  expect(xScale([5, 5]).min).toBe(4)
  expect(xScale([]).ticks).toEqual([0, 1])
})

test('the new kinds keep the point limit, and a kind that is still unknown is refused', () => {
  const many = Array.from({ length: MAX_POINTS + 1 }, (_, i) => String(i))
  const values = many.map(() => 1)
  for (const type of ['area', 'stacked', 'scatter']) expect(parseChart(payload({ type, labels: many, series: [{ name: 's', values }] })).ok).toBe(false)
  const radar = parseChart(payload({ type: 'radar' }))
  expect(radar.ok).toBe(false)
  if (!radar.ok) expect(radar.reason).toContain('area')
})
