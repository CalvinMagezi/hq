// What the sheet screen does with a grid that is not an edit of the note: the order and the rows it shows
// (the VIEW only), copy and paste of a range, fill-down, the record card of one row, and a note's cell edits
// applied in one go. Pure, so the touch and keyboard paths share one tested set of rules.
import type { Cell, Sheet } from './sheetCalc'
import { acceptCell, setCell } from './hqmdEdit'

export interface ViewSpec {
  sort: { col: number; dir: 'asc' | 'desc' } | null
  /** Rows whose column shows this text (case does not matter); null shows all. */
  filter: { col: number; text: string } | null
}
export const NO_VIEW: ViewSpec = { sort: null, filter: null }

const shown = (cell: Cell | undefined, raw: string): number | string => (cell?.ok ? (typeof cell.value === 'boolean' ? String(cell.value) : cell.value) : raw)

/** The data rows (from 0) in the order and the subset the view asks for. The note's own order is untouched. */
export function viewOrder(sheet: Sheet, cells: Cell[][], view: ViewSpec): number[] {
  let order = sheet.rows.map((_, r) => r)
  const f = view.filter
  if (f && f.text.trim() !== '') {
    const q = f.text.trim().toLowerCase()
    order = order.filter((r) => String(shown(cells[r]?.[f.col], sheet.rows[r][f.col])).toLowerCase().includes(q))
  }
  if (view.sort) {
    const { col, dir } = view.sort
    const sign = dir === 'asc' ? 1 : -1
    order = [...order].sort((a, b) => {
      const x = shown(cells[a]?.[col], sheet.rows[a][col])
      const y = shown(cells[b]?.[col], sheet.rows[b][col])
      // Empty cells go last whichever way it is sorted.
      if (x === '' || y === '') return x === y ? a - b : x === '' ? 1 : -1
      if (typeof x === 'number' && typeof y === 'number') return sign * (x - y) || a - b
      return sign * String(x).localeCompare(String(y), undefined, { numeric: true, sensitivity: 'base' }) || a - b
    })
  }
  return order
}

export interface CellBox {
  r1: number
  c1: number
  r2: number
  c2: number
}

export const normalBox = (b: CellBox): CellBox => ({ r1: Math.min(b.r1, b.r2), r2: Math.max(b.r1, b.r2), c1: Math.min(b.c1, b.c2), c2: Math.max(b.c1, b.c2) })

/** A range as tab and newline separated text, the format every spreadsheet puts on the clipboard. Formulas stay as typed. */
export function toTsv(sheet: Sheet, box: CellBox): string {
  const b = normalBox(box)
  const lines: string[] = []
  for (let r = b.r1; r <= b.r2; r += 1) {
    const cells: string[] = []
    for (let c = b.c1; c <= b.c2; c += 1) cells.push((sheet.rows[r]?.[c] ?? '').replace(/[\t\r\n]+/g, ' '))
    lines.push(cells.join('\t'))
  }
  return lines.join('\n')
}

/** Clipboard text as a grid: tabs split cells, newlines split rows, one trailing newline is ignored. */
export function parseTsv(text: string): string[][] {
  const body = text.replace(/\r\n?/g, '\n').replace(/\n$/, '')
  if (body === '') return []
  return body.split('\n').map((l) => l.split('\t'))
}

export interface CellEdit {
  row: number
  col: number
  was: string
  value: string
}

/** The edits a paste makes starting at a cell, and how many cells were refused (wrong type) or fell outside the sheet. */
export function pasteEdits(sheet: Sheet, at: { row: number; col: number }, grid: string[][]): { edits: CellEdit[]; skipped: number } {
  const edits: CellEdit[] = []
  let skipped = 0
  grid.forEach((line, dr) =>
    line.forEach((value, dc) => {
      const row = at.row + dr
      const col = at.col + dc
      if (row >= sheet.rows.length || col >= sheet.header.length || value.includes('|') || acceptCell(value, sheet.types[col]) === null) {
        skipped += 1
        return
      }
      if (sheet.rows[row][col] !== value) edits.push({ row, col, was: sheet.rows[row][col], value })
    }),
  )
  return { edits, skipped }
}

/** Moves the row numbers of the relative references in a formula by `delta`; `$B$2` and `B$2` stay put. */
export function shiftFormula(formula: string, delta: number): string {
  return formula.replace(/(\$?)([A-Z]{1,3})(\$?)(\d{1,7})(?![\d(A-Za-z])/g, (m, d1: string, col: string, d2: string, row: string, offset: number, whole: string) => {
    if (/[A-Za-z]/.test(whole[offset - 1] ?? '')) return m
    if (d2 === '$') return m
    return `${d1}${col}${d2}${Math.max(1, Number(row) + delta)}`
  })
}

/** Fill down: the top row of the box copied into the rows below it, a formula's references moving with it. */
export function fillDownEdits(sheet: Sheet, box: CellBox): CellEdit[] {
  const b = normalBox(box)
  const edits: CellEdit[] = []
  for (let c = b.c1; c <= b.c2; c += 1) {
    const top = sheet.rows[b.r1]?.[c]
    if (top === undefined) continue
    for (let r = b.r1 + 1; r <= b.r2; r += 1) {
      const value = top.startsWith('=') ? shiftFormula(top, r - b.r1) : top
      if (sheet.rows[r] && sheet.rows[r][c] !== value) edits.push({ row: r, col: c, was: sheet.rows[r][c], value })
    }
  }
  return edits
}

/** The nth sheet's cells set one after the other; null if any is stale, so none is written. */
export function setCells(text: string, sheetIndex: number, edits: CellEdit[]): string | null {
  let cur: string | null = text
  for (const e of edits) {
    cur = cur === null ? null : setCell(cur, sheetIndex, e.row, e.col, e.was, e.value)
  }
  return cur
}

export interface RecordField {
  col: number
  header: string
  type: string
  raw: string
}

/** One row as a list of label and value, for a screen too narrow for the grid. */
export function recordFields(sheet: Sheet, row: number): RecordField[] {
  return sheet.header.map((header, col) => ({ col, header, type: sheet.types[col], raw: sheet.rows[row]?.[col] ?? '' }))
}

/** Cells the local formula reader could not work out, as [row, col] (data row from 0). */
export function unworked(sheet: Sheet, cells: Cell[][]): [number, number][] {
  const out: [number, number][] = []
  sheet.rows.forEach((row, r) =>
    row.forEach((raw, c) => {
      if (raw.startsWith('=') && cells[r]?.[c] && !cells[r][c].ok) out.push([r, c])
    }),
  )
  return out
}

export const WINDOW_ROWS = 30
export const WINDOW_COLS = 10
export const MAX_WINDOWS = 6

/** The server windows (30 rows by 10 columns, header row counted at row 0) that hold these cells; at most six are asked for. */
export function windowsFor(bad: [number, number][]): { row: number; col: number }[] {
  const seen = new Map<string, { row: number; col: number }>()
  for (const [r, c] of bad) {
    const row = Math.floor((r + 1) / WINDOW_ROWS) * WINDOW_ROWS
    const col = Math.floor(c / WINDOW_COLS) * WINDOW_COLS
    seen.set(`${row}:${col}`, { row, col })
  }
  return [...seen.values()].slice(0, MAX_WINDOWS)
}
