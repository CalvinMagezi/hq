import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { ArrowDown, ArrowUp, GripVertical } from 'lucide-react'
import { t, type StringKey } from '~/strings'
import type { HqmdNote, TypedBlock } from '~/lib/hqmd'
import { formatValue, parseRange, tileValue, type Sheet } from '~/lib/sheetCalc'
import { filterColumns, plotData, tileNumbers, type ViewFilter } from '~/lib/dashQuery'
import { SIZES, moveBlock, setSize, sizeOf, type Size } from '~/lib/dashLayout'
import { PlotChart, Sparkline } from './PlotView'
import { usePointerDrag } from '../tasks/usePointerDrag'
import type { Editor } from './Editor'

const WHY: Record<string, StringKey> = {
  'other-note': 'suite_why_other_note',
  'no-sheet': 'suite_why_no_sheet',
  text: 'suite_why_text',
  columns: 'suite_why_columns',
  fn: 'suite_why_fn',
  groups: 'suite_why_groups',
}

const SIZE_WORDS: Record<Size, StringKey> = { small: 'suite_size_small', wide: 'suite_size_wide', full: 'suite_size_full' }

/** What a block is called on screen and to a screen reader: a tile's label, a plot's title, else its place. */
function nameOf(block: TypedBlock, n: number): string {
  if (block.type === 'tile') return block.attrs.label || t('suite_layout_tile_n', { n: String(n + 1) })
  return block.attrs.title || t('suite_layout_plot_n', { n: String(n + 1) })
}

type Control = 'up' | 'down' | 'grip'

/**
 * The dashboard: dropdown filters that narrow what this page draws (the note is never written), and the tiles and
 * plots in the order the note has them, each as small, wide or full width. Arrange turns on moving and sizing; both
 * are written into the note, so every device sees the same layout.
 */
export function DashboardView({ note, sheets, editor, tools }: { note: HqmdNote; sheets: Sheet[]; editor?: Editor; /** Buttons for the row above the dashboard (the comments button). */ tools?: ReactNode }) {
  const [chosen, setChosen] = useState<Record<string, string>>({})
  const [arrange, setArrange] = useState(false)
  const [say, setSay] = useState('')
  const [dragging, setDragging] = useState<{ from: number; over: number } | null>(null)
  const slots = useRef<(HTMLDivElement | null)[]>([])
  const controls = useRef(new Map<string, HTMLButtonElement>())
  const [refocus, setRefocus] = useState<{ n: number; control: Control } | null>(null)
  const items = useMemo(() => note.blocks.filter((b) => b.type === 'tile' || b.type === 'plot'), [note])
  const choices = useMemo(() => (sheets[0] ? filterColumns(sheets[0]) : []), [sheets])
  const filters: ViewFilter[] = Object.entries(chosen)
    .filter(([, v]) => v !== '')
    .map(([column, value]) => ({ column, value }))
  const canArrange = !!editor?.on && items.length > 0
  const arranging = arrange && canArrange

  // After a block moves, the control the person used is on its new place: put focus back on it.
  useEffect(() => {
    if (!refocus) return
    const last = items.length - 1
    const control = refocus.control === 'up' && refocus.n === 0 ? 'down' : refocus.control === 'down' && refocus.n === last ? 'up' : refocus.control
    controls.current.get(`${refocus.n}:${control}`)?.focus()
    setRefocus(null)
  }, [refocus, items.length])

  const move = (from: number, to: number, control: Control | null): boolean => {
    const block = items[from]
    if (!editor || !block || to < 0 || to >= items.length || to === from) return false
    const ok = editor.apply((c) => moveBlock(c, from, to, block.head))
    if (ok) {
      setSay(t('suite_layout_moved', { label: nameOf(block, from), n: String(to + 1), total: String(items.length) }))
      if (control) setRefocus({ n: to, control })
    }
    return ok
  }
  const resize = (n: number, size: Size) => {
    const block = items[n]
    if (editor && block && editor.apply((c) => setSize(c, n, block.head, size))) setSay(t('suite_layout_sized', { label: nameOf(block, n), size: t(SIZE_WORDS[size]).toLowerCase() }))
  }

  const over = (x: number, y: number): number | null => {
    const hit = slots.current.findIndex((el) => {
      const r = el?.getBoundingClientRect()
      return !!r && r.left <= x && x <= r.right && r.top <= y && y <= r.bottom
    })
    return hit < 0 || hit >= items.length ? null : hit
  }
  const startDrag = usePointerDrag<number>({
    onStart: (from) => setDragging({ from, over: from }),
    onMove: (from, p) => setDragging({ from, over: over(p.x, p.y) ?? from }),
    onEnd: (from, p) => {
      const to = over(p.x, p.y) ?? from
      setDragging(null)
      if (to !== from) move(from, to, null)
    },
    onCancel: () => setDragging(null),
  })

  if (items.length === 0) {
    return (
      <div className="su-dash">
        {tools && <div className="su-toolbar su-topbar">{tools}</div>}
        <p className="a-body-sm a-muted">{t('suite_dash_empty')}</p>
      </div>
    )
  }
  const keep = (key: string) => (el: HTMLButtonElement | null) => {
    if (el) controls.current.set(key, el)
    else controls.current.delete(key)
  }
  return (
    <div className="su-dash">
      {(tools || canArrange) && (
        <div className="su-toolbar su-topbar">
          {tools}
          {canArrange && (
            <button type="button" className="a-btn quiet compact" aria-pressed={arranging} onClick={() => setArrange(!arranging)}>
              {arranging ? t('suite_layout_done') : t('suite_layout_arrange')}
            </button>
          )}
        </div>
      )}
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
      {arranging && <p className="a-body-sm a-muted su-arrange-hint">{t('suite_layout_hint')}</p>}
      <div className={`su-board${arranging ? ' is-arranging' : ''}`}>
        {items.map((b, n) => {
          const label = nameOf(b, n)
          const size = sizeOf(b)
          const state = dragging ? (dragging.from === n ? ' is-dragging' : dragging.over === n ? ' is-over' : '') : ''
          return (
            <div
              key={`${b.type}:${n}`}
              ref={(el) => {
                slots.current[n] = el
              }}
              className={`su-slot su-size-${size}${state}`}
            >
              {b.type === 'tile' ? <Tile block={b} sheets={sheets} filters={filters} /> : <Plot block={b} sheets={sheets} filters={filters} />}
              {arranging && (
                <div className="su-slot-bar" role="group" aria-label={t('suite_layout_for', { label })}>
                  <button type="button" ref={keep(`${n}:up`)} className="a-ib sm" disabled={n === 0} aria-label={t('suite_layout_earlier', { label })} onClick={() => move(n, n - 1, 'up')}>
                    <ArrowUp className="a-ic" aria-hidden="true" />
                  </button>
                  <button type="button" ref={keep(`${n}:down`)} className="a-ib sm" disabled={n === items.length - 1} aria-label={t('suite_layout_later', { label })} onClick={() => move(n, n + 1, 'down')}>
                    <ArrowDown className="a-ic" aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    ref={keep(`${n}:grip`)}
                    className="a-ib sm su-grip"
                    aria-label={t('suite_layout_grip', { label })}
                    onPointerDown={(e) => startDrag(e, n)}
                    onKeyDown={(e) => {
                      const to = e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? n - 1 : e.key === 'ArrowDown' || e.key === 'ArrowRight' ? n + 1 : e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : null
                      if (to === null) return
                      e.preventDefault()
                      move(n, to, 'grip')
                    }}
                  >
                    <GripVertical className="a-ic" aria-hidden="true" />
                  </button>
                  <select className="su-select su-size-pick" aria-label={t('suite_layout_size', { label })} value={size} onChange={(e) => resize(n, e.target.value as Size)}>
                    {SIZES.map((s) => (
                      <option key={s} value={s}>
                        {t(SIZE_WORDS[s])}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </div>
          )
        })}
      </div>
      <p className="su-sr" role="status" aria-live="polite">
        {say}
      </p>
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
  const data = plotData(sheets, { from: a.from ?? '', x: a.x ?? '', y: a.y ?? '', where: a.where, group: kind === 'pie' || kind === 'donut' ? undefined : a.group, kind }, filters)
  return (
    <figure className="su-plot">
      {title && <figcaption className="a-body-b">{title}</figcaption>}
      {data.ok ? <PlotChart kind={kind} title={title} labels={data.labels} series={data.series} format={a.format ?? 'num'} /> : <p className="a-body-sm a-muted">{t(WHY[data.why])}</p>}
    </figure>
  )
}
