// Renders ```chart fenced blocks to inline SVG. Format: docs/CHART_BLOCKS.md.

export type ChartType = 'line' | 'bar' | 'pie' | 'donut'

export interface ChartSeries {
  name: string
  values: number[]
}

export interface ChartSpec {
  type: ChartType
  title: string
  xLabel: string
  yLabel: string
  labels: string[]
  series: ChartSeries[]
}

export type ChartParse = { ok: true; spec: ChartSpec } | { ok: false; reason: string }

const CHART_TYPES: ChartType[] = ['line', 'bar', 'pie', 'donut']
export const MAX_POINTS = 50
export const MAX_SLICES = 12
export const MAX_SERIES = 6
const MAX_TEXT = 120
const MAX_TICK_LABEL = 10

// Existing theme variables only, so the chart follows the app theme.
export const SERIES_COLORS = [
  'var(--accent-blue)',
  'var(--accent-green)',
  'var(--accent-amber)',
  'var(--text-primary)',
  'var(--accent-red)',
  'var(--text-dim)',
]

const VIEW_W = 360
const VIEW_H = 240
const MARGIN = { left: 52, right: 12, top: 10, bottom: 44 }
const Y_TICKS = 4
const POINT_MARKER_LIMIT = 30
const PIE_RADIUS = 90
const DONUT_HOLE = 0.55
const FULL_TURN = Math.PI * 2
const FULL_TURN_EPS = 0.0001

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v)

const text = (v: unknown, fallback = ''): string =>
  typeof v === 'string' || typeof v === 'number' ? String(v).slice(0, MAX_TEXT) : fallback

const fail = (reason: string): ChartParse => ({ ok: false, reason })

function parseSeries(raw: unknown, pointCount: number): ChartSeries[] | string {
  if (!Array.isArray(raw) || raw.length === 0) return 'series must be a non-empty array'
  if (raw.length > MAX_SERIES) return `at most ${MAX_SERIES} series are supported`
  const out: ChartSeries[] = []
  for (const [i, item] of raw.entries()) {
    if (!isRecord(item) || !Array.isArray(item.values)) return `series ${i + 1} needs a values array`
    const values = item.values
    if (values.length !== pointCount) return `series ${i + 1} must have ${pointCount} values, one per label`
    if (!values.every((v) => typeof v === 'number' && Number.isFinite(v))) return `series ${i + 1} has a non-numeric value`
    out.push({ name: text(item.name, `Series ${i + 1}`), values: values as number[] })
  }
  return out
}

export function parseChart(source: string): ChartParse {
  let raw: unknown
  try {
    raw = JSON.parse(source)
  } catch {
    return fail('payload is not valid JSON')
  }
  if (!isRecord(raw)) return fail('payload must be a JSON object')
  const type = CHART_TYPES.find((t) => t === raw.type)
  if (!type) return fail(`type must be one of ${CHART_TYPES.join(', ')}`)
  if (!Array.isArray(raw.labels) || raw.labels.length === 0) return fail('labels must be a non-empty array')
  const isPie = type === 'pie' || type === 'donut'
  const limit = isPie ? MAX_SLICES : MAX_POINTS
  if (raw.labels.length > limit) return fail(`at most ${limit} labels are supported for ${type} charts`)
  const labels = raw.labels.map((l) => text(l))
  const series = parseSeries(raw.series, labels.length)
  if (typeof series === 'string') return fail(series)
  if (isPie) {
    const values = series[0].values
    if (values.some((v) => v < 0)) return fail('pie and donut values must not be negative')
    if (values.reduce((a, b) => a + b, 0) <= 0) return fail('pie and donut values must sum to more than zero')
  }
  return {
    ok: true,
    spec: { type, title: text(raw.title), xLabel: text(raw.xLabel), yLabel: text(raw.yLabel), labels, series },
  }
}

const compact = (n: number) => n.toLocaleString('en', { notation: 'compact', maximumFractionDigits: 1 })
const round = (n: number) => Math.round(n * 100) / 100
const clip = (s: string) => (s.length > MAX_TICK_LABEL ? `${s.slice(0, MAX_TICK_LABEL - 1)}…` : s)

/** A round step (1, 2 or 5 times a power of ten) that gives about `count` intervals. */
export function niceStep(range: number, count: number): number {
  const rough = range / count
  const pow = 10 ** Math.floor(Math.log10(rough))
  const frac = rough / pow
  const nice = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 5 ? 5 : 10
  return nice * pow
}

export function yScale(values: number[]): { min: number; max: number; ticks: number[] } {
  const lo = Math.min(0, ...values)
  const hi = Math.max(0, ...values)
  const step = niceStep(hi === lo ? 1 : hi - lo, Y_TICKS)
  const min = Math.floor(lo / step) * step
  const max = Math.ceil(hi / step) * step
  const ticks: number[] = []
  for (let v = min; v <= max + step / 2; v += step) ticks.push(round(v))
  return { min, max: max === min ? min + step : max, ticks }
}

const axisText = (x: number, y: number, s: string, anchor: string, extra = '') =>
  `<text x="${round(x)}" y="${round(y)}" text-anchor="${anchor}" style="fill:var(--text-dim);font-size:10px" ${extra}>${escapeHtml(s)}</text>`

function cartesianSvg(spec: ChartSpec): string {
  const { left, right, top, bottom } = MARGIN
  const plotW = VIEW_W - left - right
  const plotH = VIEW_H - top - bottom
  const scale = yScale(spec.series.flatMap((s) => s.values))
  const y = (v: number) => top + (1 - (v - scale.min) / (scale.max - scale.min)) * plotH
  const band = plotW / spec.labels.length
  const cx = (i: number) => left + (i + 0.5) * band
  const parts: string[] = []

  for (const t of scale.ticks) {
    parts.push(`<line x1="${left}" x2="${VIEW_W - right}" y1="${round(y(t))}" y2="${round(y(t))}" style="stroke:var(--text-dim);opacity:0.25"/>`)
    parts.push(axisText(left - 6, y(t) + 3, compact(t), 'end'))
  }
  const every = Math.ceil(spec.labels.length / 8)
  spec.labels.forEach((l, i) => {
    if (i % every === 0) parts.push(axisText(cx(i), VIEW_H - bottom + 14, clip(l), 'middle'))
  })
  if (spec.xLabel) parts.push(axisText(left + plotW / 2, VIEW_H - 6, spec.xLabel, 'middle'))
  if (spec.yLabel) parts.push(axisText(10, top + plotH / 2, spec.yLabel, 'middle', `transform="rotate(-90 10 ${round(top + plotH / 2)})"`))

  const body = spec.type === 'bar' ? barMarks(spec, band, cx, y) : lineMarks(spec, cx, y)
  return svgWrap(spec, parts.join('') + body)
}

function barMarks(spec: ChartSpec, band: number, cx: (i: number) => number, y: (v: number) => number): string {
  const barW = (band * 0.8) / spec.series.length
  const zero = y(0)
  return spec.series
    .map((s, si) =>
      s.values
        .map((v, i) => {
          const x = cx(i) - (band * 0.8) / 2 + si * barW
          const top = Math.min(y(v), zero)
          return `<rect x="${round(x)}" y="${round(top)}" width="${round(barW)}" height="${round(Math.max(Math.abs(y(v) - zero), 1))}" style="fill:${SERIES_COLORS[si]}"/>`
        })
        .join(''),
    )
    .join('')
}

function lineMarks(spec: ChartSpec, cx: (i: number) => number, y: (v: number) => number): string {
  return spec.series
    .map((s, si) => {
      const pts = s.values.map((v, i) => `${round(cx(i))},${round(y(v))}`)
      const line = `<polyline points="${pts.join(' ')}" fill="none" style="stroke:${SERIES_COLORS[si]};stroke-width:2"/>`
      if (s.values.length > POINT_MARKER_LIMIT) return line
      const dots = s.values.map((v, i) => `<circle cx="${round(cx(i))}" cy="${round(y(v))}" r="2.5" style="fill:${SERIES_COLORS[si]}"/>`)
      return line + dots.join('')
    })
    .join('')
}

function wedge(cx: number, cy: number, r0: number, r1: number, a0: number, a1: number): string {
  const end = Math.min(a1, a0 + FULL_TURN - FULL_TURN_EPS)
  const pt = (r: number, a: number) => `${round(cx + r * Math.sin(a))},${round(cy - r * Math.cos(a))}`
  const large = end - a0 > Math.PI ? 1 : 0
  const outer = `M ${pt(r1, a0)} A ${r1} ${r1} 0 ${large} 1 ${pt(r1, end)}`
  if (r0 === 0) return `${outer} L ${cx},${cy} Z`
  return `${outer} L ${pt(r0, end)} A ${r0} ${r0} 0 ${large} 0 ${pt(r0, a0)} Z`
}

function pieSvg(spec: ChartSpec): string {
  const values = spec.series[0].values
  const total = values.reduce((a, b) => a + b, 0)
  const cx = VIEW_W / 2
  const cy = VIEW_H / 2
  const inner = spec.type === 'donut' ? PIE_RADIUS * DONUT_HOLE : 0
  let angle = 0
  const slices = values.map((v, i) => {
    const sweep = (v / total) * FULL_TURN
    const d = v > 0 ? wedge(cx, cy, inner, PIE_RADIUS, angle, angle + sweep) : ''
    angle += sweep
    return d ? `<path d="${d}" style="fill:${SERIES_COLORS[i % SERIES_COLORS.length]};stroke:var(--bg-solid-surface);stroke-width:1.5"/>` : ''
  })
  return svgWrap(spec, slices.join(''))
}

function svgWrap(spec: ChartSpec, inner: string): string {
  const label = spec.title || `${spec.type} chart`
  return `<svg viewBox="0 0 ${VIEW_W} ${VIEW_H}" role="img" aria-label="${escapeHtml(label)}" style="width:100%;height:auto;display:block"><title>${escapeHtml(label)}</title>${inner}</svg>`
}

const legendItem = (color: string, label: string) =>
  `<li style="display:flex;align-items:center;gap:6px;min-width:0"><span style="width:10px;height:10px;border-radius:2px;flex:none;background:${color}"></span><span style="overflow-wrap:anywhere">${escapeHtml(label)}</span></li>`

function legendHtml(spec: ChartSpec): string {
  if (spec.type === 'pie' || spec.type === 'donut') {
    const values = spec.series[0].values
    const total = values.reduce((a, b) => a + b, 0)
    return spec.labels
      .map((l, i) => legendItem(SERIES_COLORS[i % SERIES_COLORS.length], `${l}: ${compact(values[i])} (${Math.round((values[i] / total) * 100)}%)`))
      .join('')
  }
  if (spec.series.length < 2) return ''
  return spec.series.map((s, i) => legendItem(SERIES_COLORS[i], s.name)).join('')
}

/** The full chart, ready to place in sanitized markdown output. */
export function renderChartHtml(spec: ChartSpec): string {
  const svg = spec.type === 'pie' || spec.type === 'donut' ? pieSvg(spec) : cartesianSvg(spec)
  const title = spec.title
    ? `<figcaption style="font-weight:600;font-size:13px;margin-bottom:6px">${escapeHtml(spec.title)}</figcaption>`
    : ''
  const legend = legendHtml(spec)
  const legendBlock = legend
    ? `<ul style="list-style:none;padding:0;margin:8px 0 0;display:flex;flex-wrap:wrap;gap:4px 14px;font-size:12px;color:var(--text-dim)">${legend}</ul>`
    : ''
  return `<figure class="chart-block" style="max-width:560px;margin:12px 0">${title}${svg}${legendBlock}</figure>`
}

const MERMAID_PIE_ROW = /^\s*"([^"]+)"\s*:\s*(-?\d+(?:\.\d+)?)\s*$/

// Models often answer with a Mermaid pie; the same validation as a chart block applies.
export function parseMermaidPie(text: string): ChartParse {
  const lines = text.split('\n').filter((line) => line.trim() !== '')
  if (!/^\s*pie\b/.test(lines[0] ?? '')) return { ok: false, reason: 'not a pie diagram' }
  let title = ''
  const labels: string[] = []
  const values: number[] = []
  for (const line of lines.slice(1)) {
    const titleMatch = line.match(/^\s*title\s+(.+)$/)
    if (titleMatch) {
      title = titleMatch[1].trim()
      continue
    }
    const row = line.match(MERMAID_PIE_ROW)
    if (!row) return { ok: false, reason: 'unsupported Mermaid pie syntax' }
    labels.push(row[1])
    values.push(Number(row[2]))
  }
  return parseChart(JSON.stringify({ type: 'pie', title, labels, series: [{ name: title || 'Value', values }] }))
}
