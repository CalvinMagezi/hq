import { useEffect, useMemo, useRef, useState } from 'react'
import { t } from '~/strings'
import { colName, computeSheet, formatValue, type Sheet } from '~/lib/sheetCalc'
import { acceptCell, addRow, setCell } from '~/lib/hqmdEdit'
import {
  NO_VIEW,
  fillDownEdits,
  normalBox,
  parseTsv,
  pasteEdits,
  recordFields,
  setCells,
  toTsv,
  unworked,
  viewOrder,
  windowsFor,
  type CellBox,
  type CellEdit,
  type ViewSpec,
} from '~/lib/sheetView'
import { fetchSheetWindow } from '~/lib/suiteApi'
import { toastAlert, toastNotice, toastUndo } from '~/lib/toast'
import { Sheet as BottomSheet } from '../ui/Sheet'
import { useSuiteOps, type OpResult } from '~/lib/suiteOps'
import { useDensity } from './useDensity'
import type { Editor } from './Editor'

// A sheet draws the rows on screen a page at a time, so a long one does not slow the pane.
const PAGE_ROWS = 200
const FX = 'fx'
function within(v: number, lo: number, hi: number): boolean {
  return lo <= v && v <= hi
}
type Ref = React.RefObject<HTMLInputElement | null>
type Target = { path: string; base?: string; index: number }
type Run = { (target: Target): Promise<OpResult> }
export const LONG_PRESS_MS = 500
const MOVE_SLOP_PX = 8
export const COLUMN_TYPES = ['text', 'int', 'num', 'pct', 'bool', 'date', 'ugx', 'usd', 'eur', 'kes']

interface Pos {
  row: number
  col: number
}
const same = (a: Pos | null, b: Pos | null) => !!a && !!b && a.row === b.row && a.col === b.col

/** hq's own answer for the cells the browser's formula reader cannot work out, by `row:col` of the data. */
function useServerValues(path: string | undefined, index: number, sheet: Sheet | undefined, cells: ReturnType<typeof computeSheet>) {
  const [values, setValues] = useState<Map<string, { text: string; error: boolean; number: boolean }>>(new Map())
  const bad = useMemo(() => (sheet ? unworked(sheet, cells) : []), [sheet, cells])
  const key = useMemo(() => bad.map(([r, c]) => `${r}:${c}:${sheet?.rows[r][c]}`).join('|'), [bad, sheet])
  useEffect(() => {
    if (!path || bad.length === 0) {
      setValues(new Map())
      return
    }
    let live = true
    void Promise.all(windowsFor(bad).map((w) => fetchSheetWindow(path, index, w.row, w.col).catch(() => null))).then((views) => {
      if (!live) return
      const next = new Map<string, { text: string; error: boolean; number: boolean }>()
      for (const v of views) {
        v?.rows.forEach((row, i) =>
          row.forEach((cell, j) => {
            // The server counts the header as row 0.
            if (v.row0 + i >= 1) next.set(`${v.row0 + i - 1}:${v.col0 + j}`, { text: cell.text, error: cell.error, number: cell.number })
          }),
        )
      }
      setValues(next)
    })
    return () => {
      live = false
    }
    // `key` stands for the cells in doubt and what is typed in them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, index, key])
  return values
}

export function SheetView({ sheets, editor }: { sheets: Sheet[]; editor: Editor }) {
  const [at, setAt] = useState(0)
  const [shown, setShown] = useState(PAGE_ROWS)
  const [anchor, setAnchor] = useState<Pos | null>(null)
  const [focus, setFocus] = useState<Pos | null>(null)
  const [editing, setEditing] = useState(false)
  const [refused, setRefused] = useState<string | null>(null)
  const [view, setView] = useState<ViewSpec>(NO_VIEW)
  const [freeze, setFreeze] = useState(false)
  const [actions, setActions] = useState(false)
  const [viewOpts, setViewOpts] = useState(false)
  const [card, setCard] = useState<number | null>(null)
  const [say, setSay] = useState('')
  const [density, toggleDensity] = useDensity()
  const barRef = useRef<HTMLInputElement>(null)
  const pointer = useRef<{ x: number; y: number; timer: ReturnType<typeof setTimeout> | null; fired: boolean; kind: string }>({ x: 0, y: 0, timer: null, fired: false, kind: 'mouse' })
  const sheet = sheets[Math.min(at, sheets.length - 1)]
  const where = Math.min(at, sheets.length - 1)
  const cells = useMemo(() => (sheet ? computeSheet(sheet) : []), [sheet])
  const server = useServerValues(editor.path, where, sheet, cells)
  const order = useMemo(() => (sheet ? viewOrder(sheet, cells, view) : []), [sheet, cells, view])
  // The sheet as the view shows it: ranges, copy, paste and fill-down follow what is on screen.
  const projected = useMemo<Sheet | null>(() => (sheet ? { ...sheet, rows: order.map((r) => sheet.rows[r]) } : null), [sheet, order])
  if (!sheet || !projected) return <p className="a-body-sm a-muted">{t('suite_no_data')}</p>

  const picked = focus
  const srcRow = picked ? order[picked.row] : undefined
  const raw = picked && srcRow !== undefined ? sheet.rows[srcRow]?.[picked.col] ?? '' : ''
  const total = sheet.totalRows ?? sheet.rows.length
  const hasMore = order.length > shown || total > sheet.rows.length
  const box: CellBox | null = anchor && focus ? normalBox({ r1: anchor.row, c1: anchor.col, r2: focus.row, c2: focus.col }) : null
  const inBox = (vr: number, c: number) => !!box && within(vr, box.r1, box.r2) && within(c, box.c1, box.c2)
  const ref = (p: Pos) => `${colName(p.col)}${(order[p.row] ?? p.row) + 2}`

  const select = (p: Pos, extend = false) => {
    setFocus(p)
    if (!extend || !anchor) setAnchor(p)
    const r = order[p.row]
    const c = r === undefined ? undefined : cells[r]?.[p.col]
    const text = r === undefined ? '' : sheet.rows[r][p.col]
    setSay(t('suite_say_cell', { cell: `${colName(p.col)}${(r ?? 0) + 2}`, value: c?.ok ? formatValue(c.value, sheet.types[p.col]) : text || t('suite_say_empty') }))
  }

  /** Applies cell edits to the note with an Undo that puts the old text back. */
  const applyEdits = (edits: CellEdit[], line: string): boolean => {
    if (edits.length === 0) return true
    const mapped = edits.map((e) => ({ ...e, row: order[e.row] }))
    if (mapped.some((e) => e.row === undefined)) return false
    const ok = editor.apply((current) => setCells(current, where, mapped))
    if (ok) {
      toastUndo(
        line,
        async () => {
          editor.apply((current) => setCells(current, where, mapped.map((e) => ({ row: e.row, col: e.col, was: e.value, value: e.was }))))
        },
        t('suite_undone'),
      )
    }
    return ok
  }

  const commit = (vr: number, col: number, value: string) => {
    setEditing(false)
    const r = order[vr]
    if (r === undefined) return
    const was = sheet.rows[r][col]
    if (value === was) return
    if (acceptCell(value, sheet.types[col]) === null) {
      setRefused(t('suite_edit_bad_value', { type: sheet.types[col] }))
      return
    }
    setRefused(null)
    editor.apply((current) => setCell(current, where, r, col, was, value))
  }

  const onGridKey = (e: React.KeyboardEvent) => {
    if (editing || !picked) return
    const move = (dr: number, dc: number) => {
      e.preventDefault()
      select({ row: Math.min(order.length - 1, Math.max(0, picked.row + dr)), col: Math.min(sheet.header.length - 1, Math.max(0, picked.col + dc)) }, e.shiftKey)
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
    } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault()
      setActions(true)
    }
  }

  const cancelPress = () => {
    if (pointer.current.timer) clearTimeout(pointer.current.timer)
    pointer.current.timer = null
  }
  const press = (e: React.PointerEvent, p: Pos) => {
    pointer.current = { x: e.clientX, y: e.clientY, timer: null, fired: false, kind: e.pointerType }
    if (!editor.on) return
    pointer.current.timer = setTimeout(() => {
      pointer.current.fired = true
      select(p)
      setActions(true)
    }, LONG_PRESS_MS)
  }
  const tap = (e: React.MouseEvent, p: Pos) => {
    cancelPress()
    if (pointer.current.fired) {
      pointer.current.fired = false
      return
    }
    // A finger on the cell that is already picked opens what can be done to it.
    if (editor.on && pointer.current.kind === 'touch' && same(focus, p) && !e.shiftKey) setActions(true)
    else select(p, e.shiftKey)
  }

  const copy = async () => {
    if (!box) return
    const text = toTsv(projected, box)
    try {
      await navigator.clipboard.writeText(text)
      toastNotice(t('suite_copied'))
    } catch {
      toastAlert(t('suite_copy_blocked'))
    }
  }
  const paste = (text: string) => {
    if (!picked) return
    const { edits, skipped } = pasteEdits(projected, { row: box?.r1 ?? picked.row, col: box?.c1 ?? picked.col }, parseTsv(text))
    if (!applyEdits(edits, t('suite_pasted', { n: String(edits.length) }))) return
    if (skipped > 0) setRefused(t('suite_paste_skipped', { n: String(skipped) }))
    else setRefused(null)
  }

  const callOp = async (run: Run, done: string) => {
    if (!editor.path) return
    const r = await run({ path: editor.path, base: editor.hash(), index: where })
    if (r.ok) {
      setSay(done)
      editor.reload()
    } else if (r.reason === 'stale') {
      toastAlert(r.message || t('suite_edit_conflict'))
      editor.reload()
    } else if (r.reason === 'failed') toastAlert(r.message || t('suite_edit_failed'))
    setActions(false)
  }

  const letter = (vr: number, c: number) => `${colName(c)}${(order[vr] ?? vr) + 2}`
  const cardRow = card === null ? null : order[card]

  return (
    <div className={`su-sheet${density === 'compact' ? ' su-compact' : ''}`}>
      <FormulaBar
        key={`${picked?.row}:${picked?.col}:${raw}`}
        inputRef={barRef}
        cell={picked ? ref(picked) : ''}
        initial={raw}
        disabled={!editor.on || !picked}
        onCommit={(v) => picked && commit(picked.row, picked.col, v)}
      />
      <div className="su-toolbar" role="toolbar" aria-label={t('suite_toolbar')}>
        {editor.on && picked && (
          <button type="button" className="a-btn quiet compact" onClick={() => setActions(true)}>
            {t('suite_actions', { cell: ref(picked) })}
          </button>
        )}
        <button type="button" className="a-btn quiet compact" onClick={() => setViewOpts(true)} aria-haspopup="dialog">
          {t('suite_view_options')}
          {(view.sort || view.filter?.text) && <span className="su-pip" aria-hidden="true" />}
        </button>
        {picked && (
          <button type="button" className="a-btn quiet compact su-narrow-only" onClick={() => setCard(picked.row)}>
            {t('suite_card_open')}
          </button>
        )}
      </div>
      <div
        className="su-grid-wrap"
        tabIndex={0}
        onKeyDown={onGridKey}
        onCopy={(e) => {
          if (!box || editing) return
          e.preventDefault()
          e.clipboardData.setData('text/plain', toTsv(projected, box))
        }}
        onPaste={(e) => {
          if (!editor.on || editing || !picked) return
          e.preventDefault()
          paste(e.clipboardData.getData('text/plain'))
        }}
      >
        <table className={`su-grid${freeze ? ' su-freeze' : ''}`} role="grid" aria-label={sheet.name} aria-rowcount={order.length + 1} aria-colcount={sheet.header.length} aria-multiselectable="true">
          <thead>
            <tr role="row">
              <th scope="col" role="columnheader" className="su-corner">
                <span className="su-sr">{t('suite_corner')}</span>
              </th>
              {sheet.header.map((h, c) => {
                const sorted = view.sort?.col === c ? view.sort.dir : null
                return (
                  <th key={c} scope="col" role="columnheader" aria-sort={sorted ? (sorted === 'asc' ? 'ascending' : 'descending') : 'none'}>
                    <button
                      type="button"
                      className="su-head"
                      onClick={() => setView((v) => ({ ...v, sort: v.sort?.col !== c ? { col: c, dir: 'asc' } : v.sort.dir === 'asc' ? { col: c, dir: 'desc' } : null }))}
                    >
                      <span className="su-letter">{colName(c)}</span>
                      <span className="su-h">{h}</span>
                      <span className="su-type">
                        {sheet.types[c]}
                        {sorted && <span aria-hidden="true"> {sorted === 'asc' ? '↑' : '↓'}</span>}
                      </span>
                    </button>
                  </th>
                )
              })}
            </tr>
          </thead>
          <tbody>
            {order.slice(0, shown).map((r, vr) => (
              <tr key={r} role="row" aria-rowindex={vr + 2}>
                <th scope="row" role="rowheader">
                  <button type="button" className="su-rowno" aria-label={t('suite_row_n', { n: String(r + 2) })} onClick={() => setCard(vr)}>
                    {r + 2}
                  </button>
                </th>
                {sheet.rows[r].map((text, c) => {
                  const cell = cells[r]?.[c]
                  const formula = text.startsWith('=')
                  const theirs = !cell?.ok && formula ? server.get(`${r}:${c}`) : undefined
                  const bad = cell && !cell.ok && !(theirs && !theirs.error)
                  const number = cell?.ok ? typeof cell.value === 'number' : !!theirs?.number
                  const here = same(picked, { row: vr, col: c })
                  const selected = inBox(vr, c)
                  return (
                    <td
                      key={c}
                      id={`su-c-${vr}-${c}`}
                      role="gridcell"
                      aria-selected={selected}
                      aria-colindex={c + 2}
                      aria-readonly={!editor.on}
                      className={`${number ? 'su-num' : ''}${formula ? ' su-formula' : ''}${bad ? ' su-bad' : ''}${theirs && !theirs.error ? ' su-theirs' : ''}${here ? ' is-picked' : selected ? ' is-range' : ''}`}
                      title={bad ? t('suite_cell_formula') : theirs && !theirs.error ? t('suite_worked_out_by_hq') : undefined}
                      onPointerDown={(e) => press(e, { row: vr, col: c })}
                      onPointerMove={(e) => {
                        if (Math.hypot(e.clientX - pointer.current.x, e.clientY - pointer.current.y) > MOVE_SLOP_PX) cancelPress()
                      }}
                      onPointerUp={cancelPress}
                      onPointerCancel={cancelPress}
                      onContextMenu={(e) => {
                        if (editor.on) {
                          e.preventDefault()
                          select({ row: vr, col: c })
                          setActions(true)
                        }
                      }}
                      onClick={(e) => tap(e, { row: vr, col: c })}
                      onDoubleClick={() => {
                        if (!editor.on) return
                        select({ row: vr, col: c })
                        setEditing(true)
                      }}
                    >
                      {editing && here ? (
                        <CellInput label={t('suite_edit_cell', { cell: letter(vr, c) })} initial={text} onCommit={(v) => commit(vr, c, v)} onCancel={() => setEditing(false)} />
                      ) : bad ? (
                        text
                      ) : cell?.ok ? (
                        formatValue(cell.value, sheet.types[c])
                      ) : theirs && !theirs.error ? (
                        theirs.text
                      ) : (
                        text
                      )}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="su-sr" role="status" aria-live="polite">
        {say}
      </p>
      {refused && (
        <p role="alert" className="a-body-sm a-warn-t">
          {refused}
        </p>
      )}
      {(view.sort || view.filter?.text) && (
        <p className="a-body-sm a-muted su-more">
          {t('suite_view_only', { shown: String(order.length), total: String(sheet.rows.length) })}
          <button type="button" className="a-btn quiet compact" onClick={() => setView(NO_VIEW)}>
            {t('suite_view_reset')}
          </button>
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
          {t('suite_rows_shown', { shown: String(Math.min(shown, order.length)), total: String(total) })}
          {shown < order.length && (
            <button type="button" className="a-btn quiet compact" onClick={() => setShown(shown + PAGE_ROWS)}>
              {t('suite_load_more')}
            </button>
          )}
        </p>
      )}
      {sheets.length > 1 && (
        <div className="su-tabs" role="tablist" aria-label={t('suite_sheet_label')}>
          {sheets.map((s, i) => (
            <button
              key={s.name + i}
              type="button"
              role="tab"
              aria-selected={i === at}
              className="su-tab"
              onClick={() => {
                setAt(i)
                setShown(PAGE_ROWS)
                setAnchor(null)
                setFocus(null)
                setView(NO_VIEW)
              }}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
      {actions && picked && (
        <CellActions
          sheet={sheet}
          cell={ref(picked)}
          pos={picked}
          srcRow={srcRow ?? 0}
          box={box}
          editor={editor}
          onClose={() => setActions(false)}
          onEdit={() => {
            setActions(false)
            setTimeout(() => barRef.current?.focus(), 0)
          }}
          onCopy={() => void copy().then(() => setActions(false))}
          onPaste={paste}
          onFill={() => {
            if (!box) return
            const edits = fillDownEdits(projected, box)
            if (edits.length === 0) toastNotice(t('suite_fill_nothing'))
            else applyEdits(edits, t('suite_filled', { n: String(edits.length) }))
            setActions(false)
          }}
          onSelect={(kind) => {
            if (kind === 'column') {
              setAnchor({ row: 0, col: picked.col })
              setFocus({ row: Math.max(0, order.length - 1), col: picked.col })
            } else {
              setAnchor({ row: picked.row, col: 0 })
              setFocus({ row: picked.row, col: sheet.header.length - 1 })
            }
            setActions(false)
          }}
          onCard={() => {
            setActions(false)
            setCard(picked.row)
          }}
          callOp={callOp}
        />
      )}
      {viewOpts && (
        <BottomSheet title={t('suite_view_options')} onClose={() => setViewOpts(false)} placement="bottom">
          <div className="su-form">
            <label className="su-field">
              <span className="a-body-sm-b">{t('suite_sort_by')}</span>
              <select
                className="su-select"
                value={view.sort ? `${view.sort.col}:${view.sort.dir}` : ''}
                onChange={(e) => {
                  const [c, d] = e.target.value.split(':')
                  setView((v) => ({ ...v, sort: e.target.value ? { col: Number(c), dir: d as 'asc' | 'desc' } : null }))
                }}
              >
                <option value="">{t('suite_sort_none')}</option>
                {sheet.header.flatMap((h, c) => [
                  <option key={`${c}a`} value={`${c}:asc`}>{`${h} (${t('suite_sort_asc')})`}</option>,
                  <option key={`${c}d`} value={`${c}:desc`}>{`${h} (${t('suite_sort_desc')})`}</option>,
                ])}
              </select>
            </label>
            <label className="su-field">
              <span className="a-body-sm-b">{t('suite_filter_column')}</span>
              <select className="su-select" value={view.filter?.col ?? 0} onChange={(e) => setView((v) => ({ ...v, filter: { col: Number(e.target.value), text: v.filter?.text ?? '' } }))}>
                {sheet.header.map((h, c) => (
                  <option key={c} value={c}>
                    {h}
                  </option>
                ))}
              </select>
            </label>
            <label className="su-field">
              <span className="a-body-sm-b">{t('suite_filter_text')}</span>
              <input className="su-select" value={view.filter?.text ?? ''} onChange={(e) => setView((v) => ({ ...v, filter: { col: v.filter?.col ?? 0, text: e.target.value } }))} />
            </label>
            <p className="a-body-sm a-muted">{t('suite_view_note')}</p>
            <label className="su-check">
              <input type="checkbox" checked={freeze} onChange={(e) => setFreeze(e.target.checked)} />
              <span>{t('suite_freeze_first')}</span>
            </label>
            <label className="su-check">
              <input type="checkbox" checked={density === 'compact'} onChange={toggleDensity} />
              <span>{t('suite_density_compact')}</span>
            </label>
          </div>
        </BottomSheet>
      )}
      {card !== null && cardRow !== null && cardRow !== undefined && (
        <RecordCard
          sheet={sheet}
          row={cardRow}
          cells={cells}
          position={{ at: card, of: order.length }}
          editable={editor.on}
          onStep={(d) => setCard(Math.min(order.length - 1, Math.max(0, card + d)))}
          onCommit={(c, v) => commit(card, c, v)}
          onClose={() => setCard(null)}
        />
      )}
    </div>
  )
}

/** The formula bar: what is typed in the picked cell, kept when Enter is pressed or the field is left. */
function FormulaBar({ cell, initial, disabled, onCommit, inputRef }: { cell: string; initial: string; disabled: boolean; onCommit: (v: string) => void; inputRef: Ref }) {
  const [value, setValue] = useState(initial)
  const done = useRef(false)
  return (
    <div className="su-fx">
      <span className="su-ref" aria-hidden="true">
        {cell}
      </span>
      <span className="su-fx-mark" aria-hidden="true">
        {FX}
      </span>
      <input
        ref={inputRef}
        className="su-fx-text"
        aria-label={t('suite_formula_bar')}
        value={value}
        disabled={disabled}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        enterKeyHint="done"
        onChange={(e) => setValue(e.target.value)}
        onFocus={() => {
          done.current = false
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            done.current = true
            onCommit(value)
            e.currentTarget.blur()
          } else if (e.key === 'Escape') {
            done.current = true
            setValue(initial)
            e.currentTarget.blur()
          }
          e.stopPropagation()
        }}
        onBlur={() => {
          if (!done.current && value !== initial) onCommit(value)
        }}
      />
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

interface ActionsProps {
  sheet: Sheet
  cell: string
  pos: Pos
  srcRow: number
  box: CellBox | null
  editor: Editor
  onClose: () => void
  onEdit: () => void
  onCopy: () => void
  onPaste: (text: string) => void
  onFill: () => void
  onSelect: (kind: 'row' | 'column') => void
  onCard: () => void
  callOp(run: Run, done: string): Promise<void>
}

/** What can be done to the picked cell, in a bottom sheet. Structure changes go through HQ and are hidden where it has no route. */
function CellActions({ sheet, cell, pos, srcRow, box, editor, onClose, onEdit, onCopy, onPaste, onFill, onSelect, onCard, callOp }: ActionsProps) {
  const { ops, available } = useSuiteOps()
  const [confirm, setConfirm] = useState<'row' | 'col' | null>(null)
  const [manual, setManual] = useState<string | null>(null)
  const held = editor.waiting
  const cells = box ? (box.r2 - box.r1 + 1) * (box.c2 - box.c1 + 1) : 1
  const doPaste = async () => {
    try {
      onPaste(await navigator.clipboard.readText())
      onClose()
    } catch {
      // The browser would not give the clipboard to a button: ask for it in a field.
      setManual('')
    }
  }
  const rowsOk = available('rows') && !held
  const colsOk = available('columns') && !held
  return (
    <BottomSheet title={t('suite_actions_title', { cell })} onClose={onClose} placement="bottom">
      <div className="su-actions">
        <button type="button" className="su-act" onClick={onEdit}>
          {t('suite_act_edit')}
        </button>
        <button type="button" className="su-act" onClick={onCopy}>
          {t('suite_act_copy', { n: String(cells) })}
        </button>
        <button type="button" className="su-act" onClick={() => void doPaste()}>
          {t('suite_act_paste')}
        </button>
        <button type="button" className="su-act" disabled={!box || box.r2 === box.r1} onClick={onFill}>
          {t('suite_act_fill')}
        </button>
        <button type="button" className="su-act" onClick={() => onSelect('column')}>
          {t('suite_act_select_col')}
        </button>
        <button type="button" className="su-act" onClick={() => onSelect('row')}>
          {t('suite_act_select_row')}
        </button>
        <button type="button" className="su-act su-narrow-only" onClick={onCard}>
          {t('suite_card_open')}
        </button>
      </div>
      {manual !== null && (
        <div className="su-form">
          <label className="su-field">
            <span className="a-body-sm-b">{t('suite_paste_here')}</span>
            <textarea className="su-textarea" rows={4} value={manual} onChange={(e) => setManual(e.target.value)} />
          </label>
          <button
            type="button"
            className="a-btn compact"
            onClick={() => {
              onPaste(manual)
              onClose()
            }}
          >
            {t('suite_paste_apply')}
          </button>
        </div>
      )}
      <ColumnType sheet={sheet} col={pos.col} disabled={!available('columnType') || held} callOp={callOp} ops={ops} />
      {(rowsOk || colsOk) && (
        <div className="su-actions" role="group" aria-label={t('suite_structure')}>
          {rowsOk && (
            <>
              <button type="button" className="su-act" onClick={() => void callOp((tg) => ops.rows(tg, 'insert', srcRow), t('suite_say_row_added'))}>
                {t('suite_act_row_above')}
              </button>
              <button type="button" className="su-act" onClick={() => void callOp((tg) => ops.rows(tg, 'insert', srcRow + 1), t('suite_say_row_added'))}>
                {t('suite_act_row_below')}
              </button>
              <button
                type="button"
                className={`su-act${confirm === 'row' ? ' su-danger' : ''}`}
                onClick={() => (confirm === 'row' ? void callOp((tg) => ops.rows(tg, 'delete', srcRow), t('suite_say_row_deleted')) : setConfirm('row'))}
              >
                {confirm === 'row' ? t('suite_act_confirm') : t('suite_act_row_delete', { n: String(srcRow + 2) })}
              </button>
            </>
          )}
          {colsOk && (
            <>
              <button type="button" className="su-act" onClick={() => void callOp((tg) => ops.columns(tg, 'insert', pos.col), t('suite_say_col_added'))}>
                {t('suite_act_col_left')}
              </button>
              <button type="button" className="su-act" onClick={() => void callOp((tg) => ops.columns(tg, 'insert', pos.col + 1), t('suite_say_col_added'))}>
                {t('suite_act_col_right')}
              </button>
              <button
                type="button"
                className={`su-act${confirm === 'col' ? ' su-danger' : ''}`}
                onClick={() => (confirm === 'col' ? void callOp((tg) => ops.columns(tg, 'delete', pos.col), t('suite_say_col_deleted')) : setConfirm('col'))}
              >
                {confirm === 'col' ? t('suite_act_confirm') : t('suite_act_col_delete', { col: colName(pos.col) })}
              </button>
            </>
          )}
        </div>
      )}
      {held && <p className="a-body-sm a-muted">{t('suite_structure_waiting')}</p>}
      <p className="a-body-sm a-muted">{t('suite_undo_hint')}</p>
    </BottomSheet>
  )
}

function ColumnType({ sheet, col, disabled, callOp, ops }: { sheet: Sheet; col: number; disabled: boolean; callOp: ActionsProps['callOp']; ops: ReturnType<typeof useSuiteOps>['ops'] }) {
  if (disabled) return null
  const now = sheet.types[col]
  const kinds = COLUMN_TYPES.includes(now) ? COLUMN_TYPES : [now, ...COLUMN_TYPES]
  return (
    <label className="su-field">
      <span className="a-body-sm-b">{t('suite_col_type', { col: colName(col) })}</span>
      <select className="su-select" value={now} onChange={(e) => void callOp((tg) => ops.columnType(tg, col, e.target.value), t('suite_say_type_set', { type: e.target.value }))}>
        {kinds.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
    </label>
  )
}

/** One row as a card: a label and a field per column, for a screen too narrow for the grid. */
function RecordCard({ sheet, row, cells, position, editable, onStep, onCommit, onClose }: { sheet: Sheet; row: number; cells: ReturnType<typeof computeSheet>; position: { at: number; of: number }; editable: boolean; onStep: (d: number) => void; onCommit: (col: number, v: string) => void; onClose: () => void }) {
  return (
    <BottomSheet
      title={t('suite_card_title', { n: String(row + 2) })}
      onClose={onClose}
      placement="bottom"
      footer={
        <>
          <button type="button" className="a-btn quiet compact" disabled={position.at === 0} onClick={() => onStep(-1)}>
            {t('suite_card_prev')}
          </button>
          <button type="button" className="a-btn quiet compact" disabled={position.at >= position.of - 1} onClick={() => onStep(1)}>
            {t('suite_card_next')}
          </button>
        </>
      }
    >
      <dl className="su-record" key={row}>
        {recordFields(sheet, row).map((f) => {
          const cell = cells[row]?.[f.col]
          const shownText = cell?.ok ? formatValue(cell.value, f.type) : f.raw
          return (
            <div key={f.col} className="su-record-row">
              <dt className="a-label">
                {f.header} <span className="a-cap a-muted">{f.type}</span>
              </dt>
              <dd>
                {editable ? (
                  <input className="su-select" aria-label={f.header} defaultValue={f.raw} onBlur={(e) => e.target.value !== f.raw && onCommit(f.col, e.target.value)} />
                ) : (
                  <span>{shownText}</span>
                )}
                {editable && f.raw.startsWith('=') && <span className="a-cap a-muted">{shownText}</span>}
              </dd>
            </div>
          )
        })}
      </dl>
    </BottomSheet>
  )
}
