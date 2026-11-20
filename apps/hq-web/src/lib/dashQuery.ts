// A dashboard's `where` and `group` (docs/suite/SHEETS.md) read in the browser, plus the view-only filters a
// person picks from a dropdown. The filters never reach the note: they narrow what this page draws.
import { computeSheet, parseRange, type Cell, type Sheet } from './sheetCalc'

const OPS = ['>=', '<=', '<>', '!=', '=', '>', '<'] as const
type Op = (typeof OPS)[number]
export const MAX_CONDITIONS = 4
export const MAX_GROUPS = 6

export interface Condition {
  column: string
  op: Op
  value: number | string
}

/** `qty>30 and item=Sugar` as conditions; null when it cannot be read (no value, unknown operator, over four). */
export function parseWhere(where: string): Condition[] | null {
  const parts = where.trim().split(/\s+and\s+/i)
  if (parts.length > MAX_CONDITIONS) return null
  const out: Condition[] = []
  for (const part of parts) {
    const m = /^(.+?)\s*(>=|<=|<>|!=|=|>|<)\s*(.*)$/.exec(part.trim())
    if (!m || !m[3].trim()) return null
    const raw = m[3].trim().replace(/^"(.*)"$/, '$1')
    const n = Number(raw)
    out.push({ column: m[1].trim(), op: m[2] as Op, value: raw !== '' && Number.isFinite(n) ? n : raw })
  }
  return out
}

/** A comparison only holds between values of the same kind; `!=` also holds between different kinds. */
export function holds(cell: Cell | undefined, c: Condition): boolean {
  const v = cell?.ok ? cell.value : ''
  const same = typeof v === 'number' ? typeof c.value === 'number' : typeof c.value === 'string'
  if (!same) return c.op === '!=' || c.op === '<>'
  const a = typeof v === 'string' ? v.toLowerCase() : v
  const b = typeof c.value === 'string' ? c.value.toLowerCase() : c.value
  switch (c.op) {
    case '=': return a === b
    case '!=':
    case '<>': return a !== b
    case '>': return a > b
    case '>=': return a >= b
    case '<': return a < b
    case '<=': return a <= b
  }
}

/** A choice made in a dropdown: only rows whose `column` shows `value`. */
export interface ViewFilter {
  column: string
  value: string
}

const colOf = (sheet: Sheet, name: string) => sheet.header.findIndex((h) => h.trim().toLowerCase() === name.trim().toLowerCase())

/** The data rows (from 0) that pass the block's own `where` and the page's filters; null when the where cannot be read. */
export function matchingRows(sheet: Sheet, cells: Cell[][], where: string | undefined, filters: ViewFilter[]): number[] | null {
  const conds = where ? parseWhere(where) : []
  if (conds === null) return null
  const tests: { col: number; cond: Condition }[] = []
  for (const c of conds) {
    const col = colOf(sheet, c.column)
    if (col < 0) return null
    tests.push({ col, cond: c })
  }
  for (const f of filters) {
    const col = colOf(sheet, f.column)
    if (col >= 0) tests.push({ col, cond: { column: f.column, op: '=', value: f.value } })
  }
  const rows: number[] = []
  sheet.rows.forEach((_, r) => {
    if (tests.every((t) => holds(cells[r]?.[t.col], t.cond))) rows.push(r)
  })
  return rows
}

/** Columns worth a dropdown: text columns with two to twenty distinct values, at most three. */
export function filterColumns(sheet: Sheet): { column: string; values: string[] }[] {
  const out: { column: string; values: string[] }[] = []
  sheet.header.forEach((h, c) => {
    if (sheet.types[c] !== 'text') return
    const values = [...new Set(sheet.rows.map((r) => r[c]).filter((v) => v !== '' && !v.startsWith('=')))]
    if (values.length >= 2 && values.length <= 20 && out.length < 3) out.push({ column: h, values: values.sort((a, b) => a.localeCompare(b)) })
  })
  return out
}

export type TileReading =
  | { ok: true; numbers: number[] }
  | { ok: false; why: 'other-note' | 'no-sheet' | 'text' | 'columns' }

/** The numbers a tile adds up. With a `where` the range's last column is read, as the server does. */
export function tileNumbers(sheets: Sheet[], from: string, where: string | undefined, filters: ViewFilter[]): TileReading {
  const range = parseRange(from)
  if (!range) return { ok: false, why: 'other-note' }
  const sheet = range.sheet ? sheets.find((s) => s.name === range.sheet || s.id === range.sheet) : sheets[0]
  if (!sheet) return { ok: false, why: 'no-sheet' }
  const cells = computeSheet(sheet)
  const keep = matchingRows(sheet, cells, where, filters)
  if (keep === null) return { ok: false, why: 'columns' }
  const ok = new Set(keep)
  const lo = Math.min(range.r1, range.r2)
  const hi = Math.max(range.r1, range.r2)
  const c1 = where ? Math.max(range.c1, range.c2) : Math.min(range.c1, range.c2)
  const c2 = Math.max(range.c1, range.c2)
  const numbers: number[] = []
  for (let r = Math.max(lo, 2); r <= hi; r += 1) {
    if (!ok.has(r - 2)) continue
    for (let c = c1; c <= c2; c += 1) {
      const cell = cells[r - 2]?.[c]
      if (cell?.ok && typeof cell.value === 'number') numbers.push(cell.value)
    }
  }
  return numbers.length ? { ok: true, numbers } : { ok: false, why: 'text' }
}

export interface Series {
  name: string
  values: number[]
}
export type PlotData = { ok: true; labels: string[]; series: Series[] } | { ok: false; why: 'other-note' | 'no-sheet' | 'columns' | 'text' | 'groups' }

/** A plot's labels and one or more series. `group` splits by a column (up to six, labels in first-seen order, a missing one counts 0). */
export function plotData(sheets: Sheet[], a: { from: string; x: string; y: string; where?: string; group?: string }, filters: ViewFilter[]): PlotData {
  const range = parseRange(a.from)
  if (!range) return { ok: false, why: 'other-note' }
  const sheet = range.sheet ? sheets.find((s) => s.name === range.sheet || s.id === range.sheet) : sheets[0]
  if (!sheet) return { ok: false, why: 'no-sheet' }
  const lo = Math.min(range.c1, range.c2)
  const hi = Math.max(range.c1, range.c2)
  const inRange = (name: string) => sheet.header.findIndex((h, i) => i >= lo && i <= hi && h.trim().toLowerCase() === name.trim().toLowerCase())
  const xi = inRange(a.x)
  const yi = inRange(a.y)
  const gi = a.group ? inRange(a.group) : -1
  if (xi < 0 || yi < 0 || (a.group && gi < 0)) return { ok: false, why: 'columns' }
  const cells = computeSheet(sheet)
  const keep = matchingRows(sheet, cells, a.where, filters)
  if (keep === null) return { ok: false, why: 'columns' }
  const first = Math.max(2, Math.min(range.r1, range.r2) + (Math.min(range.r1, range.r2) <= 1 ? 1 : 0))
  const last = Math.max(range.r1, range.r2)
  const labels: string[] = []
  const groups: string[] = []
  const grid = new Map<string, number>()
  for (const r of keep) {
    if (r + 2 < first || r + 2 > last) continue
    const y = cells[r]?.[yi]
    if (!y?.ok || typeof y.value !== 'number') continue
    const lx = cells[r][xi]
    const label = lx?.ok ? String(lx.value) : String(r + 2)
    const gc = gi >= 0 ? cells[r][gi] : undefined
    const g = gi >= 0 ? (gc?.ok ? String(gc.value) : '') : a.y
    if (!labels.includes(label)) labels.push(label)
    if (!groups.includes(g)) groups.push(g)
    grid.set(`${g}\u0000${label}`, (grid.get(`${g}\u0000${label}`) ?? 0) + y.value)
  }
  if (labels.length === 0) return { ok: false, why: 'text' }
  if (groups.length > MAX_GROUPS) return { ok: false, why: 'groups' }
  return { ok: true, labels, series: groups.map((g) => ({ name: g, values: labels.map((l) => grid.get(`${g}\u0000${l}`) ?? 0) })) }
}
