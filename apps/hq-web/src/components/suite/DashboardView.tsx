import { useMemo, useState } from 'react'
import { t, type StringKey } from '~/strings'
import type { HqmdNote, TypedBlock } from '~/lib/hqmd'
import { formatValue, parseRange, tileValue, type Sheet } from '~/lib/sheetCalc'
import { filterColumns, plotData, tileNumbers, type ViewFilter } from '~/lib/dashQuery'
import { PlotChart, Sparkline } from './PlotView'

const WHY: Record<string, StringKey> = {
  'other-note': 'suite_why_other_note',
  'no-sheet': 'suite_why_no_sheet',
  text: 'suite_why_text',
  columns: 'suite_why_columns',
  fn: 'suite_why_fn',
  groups: 'suite_why_groups',
}

/** The dashboard: dropdown filters that narrow what this page draws (the note is never written), tiles and plots in a grid. */
export function DashboardView({ note, sheets }: { note: HqmdNote; sheets: Sheet[] }) {
  const [chosen, setChosen] = useState<Record<string, string>>({})
  const tiles = note.blocks.filter((b) => b.type === 'tile')
  const plots = note.blocks.filter((b) => b.type === 'plot')
  const choices = useMemo(() => (sheets[0] ? filterColumns(sheets[0]) : []), [sheets])
  const filters: ViewFilter[] = Object.entries(chosen)
    .filter(([, v]) => v !== '')
    .map(([column, value]) => ({ column, value }))
  if (tiles.length + plots.length === 0) {
    return <p className="a-body-sm a-muted">{t('suite_dash_empty')}</p>
  }
  return (
    <div className="su-dash">
      {choices.length > 0 && (
        <div className="su-filters" role="group" aria-label={t('suite_dash_filters')}>
          {choices.map((c) => (
            <label key={c.column} className="su-field">
              <span className="a-body-sm-b">{c.column}</span>
              <select className="su-select" value={chosen[c.column] ?? ''} onChange={(e) => setChosen((s) => ({ ...s, [c.column]: e.target.value }))}>
                <option value="">{t('suite_dash_all')}</option>
                {c.values.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            </label>
          ))}
          {filters.length > 0 && (
            <button type="button" className="a-btn quiet compact" onClick={() => setChosen({})}>
              {t('suite_dash_clear')}
            </button>
          )}
          <p className="a-cap a-muted su-filter-note">{t('suite_dash_view_only')}</p>
        </div>
      )}
      {tiles.length > 0 && (
        <div className="su-tiles">
          {tiles.map((b, i) => (
            <Tile key={i} block={b} sheets={sheets} filters={filters} />
          ))}
        </div>
      )}
      <div className="su-plots">
        {plots.map((b, i) => (
          <Plot key={i} block={b} sheets={sheets} filters={filters} />
        ))}
      </div>
    </div>
  )
}

function Tile({ block, sheets, filters }: { block: TypedBlock; sheets: Sheet[]; filters: ViewFilter[] }) {
  const from = block.attrs.from ?? ''
  const reading = tileNumbers(sheets, from, block.attrs.where, filters)
  const value = reading.ok ? tileValue(block.attrs.fn ?? '', reading.numbers) : null
  const label = block.attrs.label ?? block.attrs.fn ?? ''
  const type = block.attrs.format ?? 'num'
  // A tile that cannot be worked out says why, in words, and never shows a zero.
  const why = reading.ok ? (value === null ? 'fn' : null) : reading.why
  // A single column of three or more numbers has a direction: draw it and say how the last step went.
  const range = parseRange(from)
  const trend = reading.ok && !block.attrs.where && range && range.c1 === range.c2 && reading.numbers.length >= 3 ? reading.numbers : null
  const delta = trend ? trend[trend.length - 1] - trend[trend.length - 2] : null
  const sign = delta === null || delta === 0 ? '■' : Math.sign(delta) === 1 ? '▲' : '▼'
  return (
    <div className="su-tile" role="group" aria-label={value === null ? label : t('suite_tile_label', { label, value: formatValue(value, type) })}>
      <span className="a-cap a-muted">{label}</span>
      {value !== null ? <b className="su-big">{formatValue(value, type)}</b> : <span className="a-body-sm a-muted">{t(WHY[why ?? 'fn'])}</span>}
      {value !== null && trend && delta !== null && (
        <span className="su-delta a-cap">
          <span aria-hidden="true">{sign}</span> {delta > 0 ? '+' : ''}
          {formatValue(delta, type)} <span className="a-muted">{t('suite_tile_delta')}</span>
          <Sparkline values={trend} />
        </span>
      )}
    </div>
  )
}

function Plot({ block, sheets, filters }: { block: TypedBlock; sheets: Sheet[]; filters: ViewFilter[] }) {
  const kind = block.attrs.type ?? 'bar'
  const title = block.attrs.title ?? ''
  const a = block.attrs
  const data = plotData(sheets, { from: a.from ?? '', x: a.x ?? '', y: a.y ?? '', where: a.where, group: kind === 'pie' || kind === 'donut' ? undefined : a.group }, filters)
  return (
    <figure className="su-plot">
      {title && <figcaption className="a-body-b">{title}</figcaption>}
      {data.ok ? <PlotChart kind={kind} title={title} labels={data.labels} series={data.series} format={a.format ?? 'num'} /> : <p className="a-body-sm a-muted">{t(WHY[data.why])}</p>}
    </figure>
  )
}
