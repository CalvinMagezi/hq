import { expect, test } from 'bun:test'
import { MAX_POINTS, MAX_SERIES, MAX_SLICES, niceStep, parseChart, parseMermaidPie, renderChartHtml, yScale, type ChartSpec } from './chart'

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
    [payload({ type: 'radar' }), 'type must be'],
    [payload({ labels: [] }), 'labels'],
    [payload({ labels: 'abc' }), 'labels'],
    [payload({ series: [] }), 'series'],
    [payload({ series: [{ name: 'x', values: [1, 2] }] }), 'must have 3 values'],
    [payload({ series: [{ name: 'x', values: [1, 'two', 3] }] }), 'non-numeric'],
    [payload({ series: [{ name: 'x', values: [1, null, 3] }] }), 'non-numeric'],
    [payload({ series: [{ name: 'x' }] }), 'values array'],
    [payload({ type: 'pie', series: [{ name: 'x', values: [1, -1, 3] }] }), 'negative'],
    [payload({ type: 'donut', series: [{ name: 'x', values: [0, 0, 0] }] }), 'sum to more than zero'],
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
