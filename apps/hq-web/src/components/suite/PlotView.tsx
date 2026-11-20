import { useId, useState } from 'react'
import { t } from '~/strings'
import { MAX_POINTS, MAX_SLICES, SERIES_DASH, compactNumber, seriesColor, seriesDashed, yScale } from '~/lib/chart'
import type { Series } from '~/lib/dashQuery'
import { formatValue } from '~/lib/sheetCalc'

// The same frame the chat's charts use (lib/chart.ts), drawn as React so a tap can read a value.
const W = 360
const H = 240
const M = { left: 52, right: 12, top: 14, bottom: 44 }
const PW = W - M.left - M.right
const PH = H - M.top - M.bottom
const LABEL_LIMIT = 12
const TICK_CHARS = 10
const PIE_R = 90
const HOLE = 0.55

export interface PlotProps {
  kind: string
  title: string
  labels: string[]
  series: Series[]
  format: string
}

const clip = (s: string) => (s.length > TICK_CHARS ? `${s.slice(0, TICK_CHARS - 1)}…` : s)
const round = (n: number) => Math.round(n * 100) / 100

/** Bar, line, pie and donut from a plot block, with axes, value labels and a tap-to-read line. Types the plot format lacks (area, stacked, scatter) are not drawn. */
export function PlotChart({ kind, title, labels, series, format }: PlotProps) {
  const [active, setActive] = useState<number | null>(null)
  const id = useId()
  const pie = kind === 'pie' || kind === 'donut'
  const cap = pie ? MAX_SLICES : MAX_POINTS
  if (cap < labels.length) return <p className="a-body-sm a-muted">{t('suite_plot_too_many', { limit: String(cap) })}</p>
  const fmt = (v: number) => formatValue(v, format)
  const summary = labels.map((l, i) => `${l} ${series.map((s) => fmt(s.values[i])).join(' ')}`).join(', ')
  const reading = active === null ? null : `${labels[active]}: ${series.map((s) => (series.length > 1 ? `${s.name} ${fmt(s.values[active])}` : fmt(s.values[active]))).join(', ')}`

  return (
    <div className="su-chart">
      {pie ? (
        <Pie id={id} kind={kind} labels={labels} values={series[0].values} active={active} setActive={setActive} aria={`${title} ${summary}`.trim()} />
      ) : (
        <Cartesian id={id} kind={kind} labels={labels} series={series} active={active} setActive={setActive} fmt={fmt} aria={`${title} ${summary}`.trim()} />
      )}
      <p className="a-body-sm su-reading" role="status" aria-live="polite">
        {reading ?? <span className="a-muted">{t('suite_plot_tap')}</span>}
      </p>
      <ul className="su-legend">
        {(pie ? labels : series.map((s) => s.name)).map((name, i) => (
          <li key={i}>
            <svg className="su-key" viewBox="0 0 16 10" aria-hidden="true">
              {kind === 'line' ? <line x1="0" x2="16" y1="5" y2="5" stroke={seriesColor(i)} strokeWidth="2" strokeDasharray={seriesDashed(i) ? SERIES_DASH : undefined} /> : <rect width="16" height="10" rx="2" fill={seriesColor(i)} />}
            </svg>
            <span className="su-legend-name">{name}</span>
            {pie && <span className="a-cap a-muted">{fmt(series[0].values[i])}</span>}
          </li>
        ))}
      </ul>
    </div>
  )
}

function Cartesian({ id, kind, labels, series, active, setActive, fmt, aria }: { id: string; kind: string; labels: string[]; series: Series[]; active: number | null; setActive: (n: number | null) => void; fmt: (v: number) => string; aria: string }) {
  const all = series.flatMap((s) => s.values)
  const scale = yScale(all)
  const y = (v: number) => M.top + (1 - (v - scale.min) / (scale.max - scale.min)) * PH
  const band = PW / labels.length
  const cx = (i: number) => M.left + (i + 0.5) * band
  const every = Math.ceil(labels.length / 6)
  const labelled = labels.length * series.length <= LABEL_LIMIT
  const barW = (band * 0.8) / series.length
  const near = (clientX: number, el: SVGSVGElement) => {
    const box = el.getBoundingClientRect()
    const x = ((clientX - box.left) / box.width) * W
    return Math.min(labels.length - 1, Math.max(0, Math.floor((x - M.left) / band)))
  }
  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      className="su-svg"
      role="img"
      aria-label={t('suite_chart_label', { kind, title: aria })}
      aria-describedby={`${id}-help`}
      tabIndex={0}
      onPointerDown={(e) => setActive(near(e.clientX, e.currentTarget))}
      onKeyDown={(e) => {
        if (e.key === 'ArrowRight') setActive(Math.min(labels.length - 1, (active ?? -1) + 1))
        else if (e.key === 'ArrowLeft') setActive(Math.max(0, (active ?? labels.length) - 1))
        else if (e.key === 'Escape') setActive(null)
        else return
        e.preventDefault()
      }}
    >
      <desc id={`${id}-help`}>{t('suite_plot_keys')}</desc>
      {scale.ticks.map((tick) => (
        <g key={tick}>
          <line x1={M.left} x2={W - M.right} y1={round(y(tick))} y2={round(y(tick))} className="su-grid-line" />
          <text x={M.left - 6} y={round(y(tick) + 3)} textAnchor="end" className="su-axis">
            {compactNumber(tick)}
          </text>
        </g>
      ))}
      <line x1={M.left} x2={W - M.right} y1={round(y(0))} y2={round(y(0))} className="su-zero" />
      {labels.map((l, i) => {
        if (i % every !== 0) return null
        return (
          <text key={i} x={round(cx(i))} y={H - M.bottom + 14} textAnchor="middle" className="su-axis">
            {clip(l)}
          </text>
        )
      })}
      {active !== null && <rect x={round(M.left + active * band)} y={M.top} width={round(band)} height={PH} className="su-active" />}
      {kind === 'bar' &&
        series.map((s, si) =>
          s.values.map((v, i) => {
            const x = cx(i) - (band * 0.8) / 2 + si * barW
            const top = Math.min(y(v), y(0))
            return (
              <g key={`${si}-${i}`}>
                <rect x={round(x)} y={round(top)} width={round(barW)} height={round(Math.max(Math.abs(y(v) - y(0)), 1))} fill={seriesColor(si)} />
                {labelled && (
                  <text x={round(x + barW / 2)} y={round(top - 3)} textAnchor="middle" className="su-vlabel">
                    {compactNumber(v)}
                  </text>
                )}
              </g>
            )
          }),
        )}
      {kind === 'line' &&
        series.map((s, si) => (
          <g key={si}>
            <polyline points={s.values.map((v, i) => `${round(cx(i))},${round(y(v))}`).join(' ')} fill="none" stroke={seriesColor(si)} strokeWidth="2" strokeDasharray={seriesDashed(si) ? SERIES_DASH : undefined} />
            {s.values.length <= 30 && s.values.map((v, i) => <circle key={i} cx={round(cx(i))} cy={round(y(v))} r={active === i ? 4.5 : 2.5} fill={seriesColor(si)} />)}
            {active === null && (
              <text x={round(cx(s.values.length - 1))} y={round(y(s.values[s.values.length - 1]) - 6)} textAnchor="middle" className="su-vlabel">
                {fmt(s.values[s.values.length - 1])}
              </text>
            )}
          </g>
        ))}
    </svg>
  )
}

function Pie({ id, kind, labels, values, active, setActive, aria }: { id: string; kind: string; labels: string[]; values: number[]; active: number | null; setActive: (n: number | null) => void; aria: string }) {
  const total = values.reduce((a, b) => a + b, 0)
  const cx = W / 2
  const cy = H / 2
  let angle = 0
  const pt = (r: number, a: number) => `${round(cx + r * Math.sin(a))},${round(cy - r * Math.cos(a))}`
  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      className="su-svg"
      role="img"
      aria-label={t('suite_chart_label', { kind, title: aria })}
      aria-describedby={`${id}-help`}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'ArrowRight') setActive(Math.min(labels.length - 1, (active ?? -1) + 1))
        else if (e.key === 'ArrowLeft') setActive(Math.max(0, (active ?? labels.length) - 1))
        else if (e.key === 'Escape') setActive(null)
        else return
        e.preventDefault()
      }}
    >
      <desc id={`${id}-help`}>{t('suite_plot_keys')}</desc>
      {values.map((v, i) => {
        const sweep = total > 0 ? (v / total) * Math.PI * 2 : 0
        const a0 = angle
        const a1 = Math.min(a0 + sweep, a0 + Math.PI * 2 - 0.0001)
        angle += sweep
        if (v <= 0) return null
        const r0 = kind === 'donut' ? PIE_R * HOLE : 0
        const large = a1 - a0 > Math.PI ? 1 : 0
        const outer = `M ${pt(PIE_R, a0)} A ${PIE_R} ${PIE_R} 0 ${large} 1 ${pt(PIE_R, a1)}`
        const d = r0 === 0 ? `${outer} L ${cx},${cy} Z` : `${outer} L ${pt(r0, a1)} A ${r0} ${r0} 0 ${large} 0 ${pt(r0, a0)} Z`
        return <path key={i} d={d} fill={seriesColor(i)} className={`su-slice${active === i ? ' is-on' : ''}`} onPointerDown={() => setActive(i)} />
      })}
    </svg>
  )
}

/** A line of numbers as a small picture, with no axes. The last point is marked. */
export function Sparkline({ values }: { values: number[] }) {
  const w = 96
  const h = 28
  const lo = Math.min(...values)
  const hi = Math.max(...values)
  const span = hi === lo ? 1 : hi - lo
  const pts = values.map((v, i) => [round((i / Math.max(values.length - 1, 1)) * (w - 6) + 3), round(h - 4 - ((v - lo) / span) * (h - 8))])
  const last = pts[pts.length - 1]
  return (
    <svg className="su-spark" viewBox={`0 0 ${w} ${h}`} aria-hidden="true">
      <polyline points={pts.map((p) => p.join(',')).join(' ')} fill="none" stroke={seriesColor(0)} strokeWidth="2" />
      <circle cx={last[0]} cy={last[1]} r="3" fill={seriesColor(0)} />
    </svg>
  )
}
