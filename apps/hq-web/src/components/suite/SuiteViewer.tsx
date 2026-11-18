import { useEffect, useMemo, useRef, useState } from 'react'
import { t, type StringKey } from '~/strings'
import { readHqmd, type HqmdNote, type TypedBlock } from '~/lib/hqmd'
import { colName, computeSheet, formatValue, readPlot, readRange, readSheet, tileValue, type Sheet } from '~/lib/sheetCalc'
import { acceptCell, addRow, setCell, setSlideBody, setSlideTitle } from '~/lib/hqmdEdit'
import { MarkdownViewer } from '../MarkdownViewer'
import { useSuiteEdit, type SaveState } from './useSuiteEdit'
import './suite.css'

const WHY: Record<string, StringKey> = {
  'other-note': 'suite_why_other_note',
  'no-sheet': 'suite_why_no_sheet',
  text: 'suite_why_text',
  columns: 'suite_why_columns',
  fn: 'suite_why_fn',
}

// A sheet draws the rows on screen and a page at a time, so a long one does not slow the pane.
const PAGE_ROWS = 200
// The formula bar's mark: a symbol, not a sentence.
const FX = 'fx'

function sheetsOf(note: HqmdNote): Sheet[] {
  return note.blocks.map(readSheet).filter((s): s is Sheet => s !== null)
}

interface ViewerProps {
  /** The whole note, frontmatter included. */
  text: string
  /** Where it lives. With it a sheet's cells and a card's title and text can be changed; without it the viewer reads. */
  path?: string
  hash?: string
  onSaved?: () => void
}

/** What the viewers use to change the note: the one function that applies an edit, and whether editing is on. */
interface Editor {
  on: boolean
  apply: (change: (current: string) => string | null) => boolean
}

/** Draws a native sheet, slides or dashboard note. Anything else is the caller's to draw as markdown. */
export function SuiteViewer({ text: source, path, hash, onSaved }: ViewerProps) {
  const saving = useSuiteEdit({ path, content: source, hash, onSaved })
  const text = saving.text
  const editor: Editor = { on: saving.canEdit, apply: saving.apply }
  const note = useMemo(() => readHqmd(text), [text])
  const sheets = useMemo(() => sheetsOf(note), [note])
  return (
    <div className="su-view">
      {note.fidelity === 'lossy' && <p className="a-body-sm a-muted su-note">{t('suite_fidelity_lossy')}</p>}
      {note.fidelity === 'view' && <p className="a-body-sm a-muted su-note">{t('suite_fidelity_view')}</p>}
      {note.kind === 'sheet' && <SheetView sheets={sheets} editor={editor} />}
      {note.kind === 'slides' && <SlidesView note={note} editor={editor} />}
      {note.kind === 'dashboard' && <DashboardView note={note} sheets={sheets} />}
      <SaveLine state={saving.state} />
    </div>
  )
}

/** One quiet line that says whether the last change reached HQ. */
function SaveLine({ state }: { state: SaveState }) {
  return (
    <p className="a-cap a-muted su-save" role="status">
      {state === 'saving' ? t('suite_edit_saving') : state === 'saved' ? t('suite_edit_saved') : ''}
    </p>
  )
}

function SheetView({ sheets, editor }: { sheets: Sheet[]; editor: Editor }) {
  const [at, setAt] = useState(0)
  const [shown, setShown] = useState(PAGE_ROWS)
  const [picked, setPicked] = useState<{ row: number; col: number } | null>(null)
  const [editing, setEditing] = useState(false)
  const [refused, setRefused] = useState<string | null>(null)
  const sheet = sheets[Math.min(at, sheets.length - 1)]
  const cells = useMemo(() => (sheet ? computeSheet(sheet) : []), [sheet])
  if (!sheet) {
    return <p className="a-body-sm a-muted">{t('suite_no_data')}</p>
  }
  const total = sheet.totalRows ?? sheet.rows.length
  const raw = picked ? sheet.rows[picked.row]?.[picked.col] : ''
  const hasMore = total > shown
  // The sheet's place among the note's sheets, for the edit that names it.
  const where = Math.min(at, sheets.length - 1)
  const commit = (row: number, col: number, value: string) => {
    setEditing(false)
    const was = sheet.rows[row][col]
    if (value === was) return
    if (acceptCell(value, sheet.types[col]) === null) {
      setRefused(t('suite_edit_bad_value', { type: sheet.types[col] }))
      return
    }
    setRefused(null)
    editor.apply((current) => setCell(current, where, row, col, was, value))
  }
  const onGridKey = (e: React.KeyboardEvent) => {
    if (editing || !picked) return
    const move = (dr: number, dc: number) => {
      e.preventDefault()
      setPicked({ row: Math.min(sheet.rows.length - 1, Math.max(0, picked.row + dr)), col: Math.min(sheet.header.length - 1, Math.max(0, picked.col + dc)) })
    }
    if (e.key === 'ArrowDown') move(1, 0)
    else if (e.key === 'ArrowUp') move(-1, 0)
    else if (e.key === 'ArrowLeft') move(0, -1)
    else if (e.key === 'ArrowRight') move(0, 1)
    else if (editor.on && (e.key === 'Enter' || e.key === 'F2')) {
      e.preventDefault()
      setEditing(true)
    } else if (editor.on && (e.key === 'Delete' || e.key === 'Backspace')) {
      e.preventDefault()
      commit(picked.row, picked.col, '')
    }
  }
  const canLoad = sheet.rows.length > shown
  return (
    <div className="su-sheet">
      <div className="su-fx">
        <span className="su-ref" aria-hidden="true">{picked ? `${colName(picked.col)}${picked.row + 2}` : ''}</span>
        <span className="su-fx-mark">{FX}</span>
        <span className="su-fx-text" aria-label={t('suite_formula_bar')}>{raw}</span>
      </div>
      <div className="su-grid-wrap" tabIndex={0} onKeyDown={onGridKey}>
        <table className="su-grid">
          <thead>
            <tr>
              <th scope="col" className="su-corner" />
              {sheet.header.map((h, c) => (
                <th key={c} scope="col">
                  <span className="su-letter">{colName(c)}</span>
                  {h}
                  <span className="su-type">{sheet.types[c]}</span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {sheet.rows.slice(0, shown).map((row, r) => (
              <tr key={r}>
                <th scope="row">{r + 2}</th>
                {row.map((raw, c) => {
                  const cell = cells[r]?.[c]
                  const formula = raw.startsWith('=')
                  const bad = cell && !cell.ok
                  const number = cell?.ok && typeof cell.value === 'number'
                  return (
                    <td
                      key={c}
                      className={`${number ? 'su-num' : ''}${formula ? ' su-formula' : ''}${bad ? ' su-bad' : ''}${picked?.row === r && picked.col === c ? ' is-picked' : ''}`}
                      title={bad ? t('suite_cell_formula') : undefined}
                      onClick={() => setPicked({ row: r, col: c })}
                      onDoubleClick={() => {
                        if (!editor.on) return
                        setPicked({ row: r, col: c })
                        setEditing(true)
                      }}
                    >
                      {editing && picked?.row === r && picked.col === c ? (
                        <CellInput label={t('suite_edit_cell', { cell: `${colName(c)}${r + 2}` })} initial={raw} onCommit={(v) => commit(r, c, v)} onCancel={() => setEditing(false)} />
                      ) : bad ? (
                        raw
                      ) : cell?.ok ? (
                        formatValue(cell.value, sheet.types[c])
                      ) : (
                        raw
                      )}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {refused && (
        <p role="alert" className="a-body-sm a-warn-t">
          {refused}
        </p>
      )}
      {editor.on && sheet.totalRows === null && (
        <div>
          <button type="button" className="a-btn quiet compact" onClick={() => editor.apply((current) => addRow(current, where))}>
            {t('suite_edit_add_row')}
          </button>
        </div>
      )}
      {hasMore && (
        <p className="a-body-sm a-muted su-more">
          {t('suite_rows_shown', { shown: String(Math.min(shown, sheet.rows.length)), total: String(total) })}
          {canLoad && (
            <button type="button" className="a-btn quiet compact" onClick={() => setShown(shown + PAGE_ROWS)}>
              {t('suite_load_more')}
            </button>
          )}
        </p>
      )}
      {sheets.length > 1 && (
        <div className="su-tabs" role="tablist" aria-label={t('suite_sheet_label')}>
          {sheets.map((s, i) => (
            <button key={s.name + i} type="button" role="tab" aria-selected={i === at} className="su-tab" onClick={() => { setAt(i); setShown(PAGE_ROWS); setPicked(null) }}>
              {s.name}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

/** The box in a cell being changed: Enter keeps it, Escape leaves it as it was, leaving the cell keeps it. */
function CellInput({ initial, label, onCommit, onCancel }: { initial: string; label: string; onCommit: (value: string) => void; onCancel: () => void }) {
  const [value, setValue] = useState(initial)
  const done = useRef(false)
  const finish = (keep: boolean) => {
    if (done.current) return
    done.current = true
    if (keep) onCommit(value)
    else onCancel()
  }
  return (
    <input
      autoFocus
      className="su-input"
      aria-label={label}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          finish(true)
        } else if (e.key === 'Escape') {
          e.preventDefault()
          finish(false)
        }
        e.stopPropagation()
      }}
      onBlur={() => finish(true)}
    />
  )
}

function SlidesView({ note, editor }: { note: HqmdNote; editor: Editor }) {
  const [at, setAt] = useState(0)
  const [editingText, setEditingText] = useState(false)
  const [draft, setDraft] = useState('')
  const [presenting, setPresenting] = useState(false)
  if (note.slides.length === 0) {
    return <p className="a-body-sm a-muted">{t('suite_no_slides')}</p>
  }
  const i = Math.min(at, note.slides.length - 1)
  const slide = note.slides[i]
  const go = (to: number) => {
    setEditingText(false)
    setAt(Math.min(note.slides.length - 1, Math.max(0, to)))
  }
  const rename = (title: string) => {
    if (title.trim() !== '' && title !== slide.title) editor.apply((current) => setSlideTitle(current, i, slide.title, title))
  }
  return (
    <div
      className="su-slides"
      tabIndex={0}
      aria-label={t('suite_slides_label')}
      onKeyDown={(e) => {
        // Arrow keys inside a field move its caret, not the slide.
        if ((e.target as HTMLElement).closest('input, textarea')) return
        if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
          e.preventDefault()
          go(i + 1)
        } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
          e.preventDefault()
          go(i - 1)
        }
      }}
    >
      <ol className="su-strip" aria-label={t('suite_slides_label')}>
        {note.slides.map((s, n) => (
          <li key={n}>
            <button type="button" className="su-thumb" aria-current={n === i ? 'true' : undefined} onClick={() => go(n)}>
              <span className="a-cap a-muted">{n + 1}</span>
              {s.title}
            </button>
          </li>
        ))}
      </ol>
      <section className={`su-stage${slide.layout ? ` su-${slide.layout}` : ''}`} aria-label={t('suite_slide_n', { n: String(i + 1), total: String(note.slides.length) })}>
        {editor.on ? <TitleInput key={`${i}:${slide.title}`} value={slide.title} label={t('suite_edit_title')} onCommit={rename} /> : <h3 className="a-h2">{slide.title}</h3>}
        {editingText ? (
          <div className="su-body-edit">
            <textarea className="su-textarea" aria-label={t('suite_edit_text_label')} value={draft} rows={8} onChange={(e) => setDraft(e.target.value)} />
            <div className="su-step">
              <button type="button" className="a-btn quiet compact" onClick={() => setEditingText(false)}>
                {t('suite_edit_cancel')}
              </button>
              <button
                type="button"
                className="a-btn secondary compact"
                onClick={() => {
                  setEditingText(false)
                  editor.apply((current) => setSlideBody(current, i, slide.body, draft))
                }}
              >
                {t('suite_edit_done')}
              </button>
            </div>
          </div>
        ) : (
          <>
            {slide.body && <MarkdownViewer content={slide.body} bare />}
            {editor.on && (
              <div>
                <button
                  type="button"
                  className="a-btn quiet compact"
                  onClick={() => {
                    setDraft(slide.body)
                    setEditingText(true)
                  }}
                >
                  {t('suite_edit_text')}
                </button>
              </div>
            )}
          </>
        )}
        <div className="su-step">
          <button type="button" className="a-btn quiet compact" onClick={() => setPresenting(true)}>
            {t('suite_present')}
          </button>
          <button type="button" className="a-btn quiet compact" disabled={i === 0} onClick={() => go(i - 1)}>
            {t('suite_slide_prev')}
          </button>
          <span className="a-cap a-muted">{t('suite_slide_n', { n: String(i + 1), total: String(note.slides.length) })}</span>
          <button type="button" className="a-btn quiet compact" disabled={i === note.slides.length - 1} onClick={() => go(i + 1)}>
            {t('suite_slide_next')}
          </button>
        </div>
      </section>
      {presenting && <Presenter slides={note.slides} start={i} onExit={() => setPresenting(false)} />}
    </div>
  )
}

/** A slide at full window: Arrow keys, Space and Page keys move, Escape leaves. Nothing is saved or changed. */
function Presenter({ slides, start, onExit }: { slides: HqmdNote['slides']; start: number; onExit: () => void }) {
  const [at, setAt] = useState(start)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onExit()
      else if (['ArrowRight', 'ArrowDown', ' ', 'PageDown'].includes(e.key)) setAt((n) => Math.min(slides.length - 1, n + 1))
      else if (['ArrowLeft', 'ArrowUp', 'PageUp'].includes(e.key)) setAt((n) => Math.max(0, n - 1))
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [slides.length, onExit])
  const slide = slides[at]
  return (
    <div className="su-present" role="dialog" aria-modal="true" aria-label={t('suite_present_label')}>
      <button type="button" className="a-btn quiet compact su-present-exit" onClick={onExit}>
        {t('suite_present_exit')}
      </button>
      <h2 className="su-present-title">{slide.title}</h2>
      {slide.body && <MarkdownViewer content={slide.body} bare />}
      <p className="a-cap a-muted su-present-n">{t('suite_slide_n', { n: String(at + 1), total: String(slides.length) })}</p>
    </div>
  )
}

/** A card's title, written in place: it is kept when the person leaves the field or presses Enter. */
function TitleInput({ value, label, onCommit }: { value: string; label: string; onCommit: (value: string) => void }) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  return (
    <input
      className="su-title-input a-h2"
      aria-label={label}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onCommit(text)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur()
        else if (e.key === 'Escape') {
          setText(value)
          e.currentTarget.blur()
        }
      }}
    />
  )
}

function DashboardView({ note, sheets }: { note: HqmdNote; sheets: Sheet[] }) {
  const tiles = note.blocks.filter((b) => b.type === 'tile')
  const plots = note.blocks.filter((b) => b.type === 'plot')
  if (tiles.length + plots.length === 0) {
    return <p className="a-body-sm a-muted">{t('suite_dash_empty')}</p>
  }
  return (
    <div className="su-dash">
      {tiles.length > 0 && (
        <div className="su-tiles">
          {tiles.map((b, i) => (
            <Tile key={i} block={b} sheets={sheets} />
          ))}
        </div>
      )}
      {plots.map((b, i) => (
        <Plot key={i} block={b} sheets={sheets} />
      ))}
    </div>
  )
}

function Tile({ block, sheets }: { block: TypedBlock; sheets: Sheet[] }) {
  const reading = readRange(sheets, block.attrs.from ?? '')
  const value = reading.ok ? tileValue(block.attrs.fn ?? '', reading.numbers) : null
  const label = block.attrs.label ?? block.attrs.fn ?? ''
  // The column the range sits in gives the display type when the tile names none.
  const type = block.attrs.format ?? 'num'
  // A tile that cannot be worked out says why, in words, and never shows a zero.
  const why = reading.ok ? (value === null ? 'fn' : null) : reading.why
  return (
    <div className="su-tile" role="group" aria-label={value === null ? label : t('suite_tile_label', { label, value: formatValue(value, type) })}>
      <span className="a-cap a-muted">{label}</span>
      {value !== null ? <b className="su-big">{formatValue(value, type)}</b> : <span className="a-body-sm a-muted">{t(WHY[why ?? 'fn'])}</span>}
    </div>
  )
}

const W = 320
const H = 160
const PIE = ['var(--brand-blue)', 'var(--brand-green)', 'var(--warn)', 'var(--ink-muted)', 'var(--line)']

function Plot({ block, sheets }: { block: TypedBlock; sheets: Sheet[] }) {
  const kind = block.attrs.type ?? 'bar'
  const title = block.attrs.title ?? ''
  const reading = readPlot(sheets, block.attrs.from ?? '', block.attrs.x ?? '', block.attrs.y ?? '')
  if (!reading.ok) {
    return (
      <figure className="su-plot">
        {title && <figcaption className="a-body-b">{title}</figcaption>}
        <p className="a-body-sm a-muted">{t(WHY[reading.why])}</p>
      </figure>
    )
  }
  const { labels, values } = reading
  const max = Math.max(...values, 1)
  const total = values.reduce((a, b) => a + b, 0)
  const summary = labels.map((l, i) => `${l} ${values[i]}`).join(', ')
  return (
    <figure className="su-plot">
      {title && <figcaption className="a-body-b">{title}</figcaption>}
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={t('suite_chart_label', { kind, title: `${title} ${summary}`.trim() })} className="su-svg">
        {kind === 'line' && (
          <polyline
            fill="none"
            stroke="var(--brand-blue)"
            strokeWidth="2"
            points={values.map((v, i) => `${(i / Math.max(values.length - 1, 1)) * (W - 20) + 10},${H - 24 - (v / max) * (H - 40)}`).join(' ')}
          />
        )}
        {kind === 'bar' &&
          values.map((v, i) => {
            const w = (W - 20) / values.length
            const h = (v / max) * (H - 40)
            return <rect key={i} x={10 + i * w + w * 0.15} y={H - 24 - h} width={w * 0.7} height={h} rx="3" fill="var(--brand-blue)" />
          })}
        {(kind === 'pie' || kind === 'donut') && <Slices values={values} total={total} donut={kind === 'donut'} />}
      </svg>
      <ul className="su-legend">
        {labels.map((l, i) => (
          <li key={i}>
            {(kind === 'pie' || kind === 'donut') && <span className="su-key" style={{ background: PIE[i % PIE.length] }} aria-hidden="true" />}
            {l}
            <span className="a-cap a-muted">{formatValue(values[i], block.attrs.format ?? 'num')}</span>
          </li>
        ))}
      </ul>
    </figure>
  )
}

function Slices({ values, total, donut }: { values: number[]; total: number; donut: boolean }) {
  const cx = W / 2
  const cy = H / 2
  const r = H / 2 - 8
  let angle = -Math.PI / 2
  return (
    <>
      {values.map((v, i) => {
        const sweep = total > 0 ? (v / total) * Math.PI * 2 : 0
        const x1 = cx + r * Math.cos(angle)
        const y1 = cy + r * Math.sin(angle)
        angle += sweep
        const x2 = cx + r * Math.cos(angle)
        const y2 = cy + r * Math.sin(angle)
        const d = `M${cx},${cy} L${x1},${y1} A${r},${r} 0 ${sweep > Math.PI ? 1 : 0} 1 ${x2},${y2} Z`
        return <path key={i} d={d} fill={PIE[i % PIE.length]} stroke="var(--surface-000)" strokeWidth="1" />
      })}
      {donut && <circle cx={cx} cy={cy} r={r * 0.55} fill="var(--surface-000)" />}
    </>
  )
}
