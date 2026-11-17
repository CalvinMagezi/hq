// The numbers of an hqmd sheet: its table, the display of each column type, and a small, bounded formula
// reader. A formula it cannot read is shown as written with the reason, never as a zero.
import type { TypedBlock } from './hqmd'

export interface Sheet {
  name: string
  id: string | null
  header: string[]
  /** One type per column: text, int, num, bool, date, pct or a currency code. */
  types: string[]
  /** Raw cell text, rows by columns. */
  rows: string[][]
  /** Total rows when the rest live in a sidecar the web does not read. */
  totalRows: number | null
}

const MAX_CELLS = 100_000

function splitRow(line: string): string[] {
  const t = line.trim().replace(/^\|/, '').replace(/\|$/, '')
  const cells: string[] = []
  let cur = ''
  for (let i = 0; i < t.length; i += 1) {
    if (t[i] === '\\' && (t[i + 1] === '|' || t[i + 1] === '\\')) {
      cur += t[i + 1]
      i += 1
    } else if (t[i] === '|') {
      cells.push(cur.trim())
      cur = ''
    } else cur += t[i]
  }
  cells.push(cur.trim())
  return cells
}

/** The sheet in a `sheet` block, or null when its table is not one (no delimiter row, uneven rows). */
export function readSheet(block: TypedBlock): Sheet | null {
  if (block.type !== 'sheet') return null
  const lines = block.body.split(/\r?\n/).filter((l) => l.trim() !== '')
  if (lines.length < 2 || !/^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/.test(lines[1].trim())) return null
  const header = splitRow(lines[0])
  const rows = lines.slice(2).map(splitRow)
  if (rows.some((r) => r.length !== header.length) || header.length * rows.length > MAX_CELLS) return null
  const given = (block.attrs.types ?? '').split(',').map((s) => s.trim())
  return {
    name: block.attrs.name ?? 'Sheet',
    id: block.attrs.id ?? null,
    header,
    types: header.map((_, i) => given[i] || 'text'),
    rows,
    totalRows: block.attrs.rows && /^\d+$/.test(block.attrs.rows) ? Number(block.attrs.rows) : null,
  }
}

export function colName(i: number): string {
  let n = i
  let s = ''
  do {
    s = String.fromCharCode(65 + (n % 26)) + s
    n = Math.floor(n / 26) - 1
  } while (n >= 0)
  return s
}

function colIndex(letters: string): number {
  return [...letters].reduce((n, c) => n * 26 + (c.charCodeAt(0) - 64), 0) - 1
}

const isCurrency = (type: string) => /^[a-z]{3}$/.test(type) && !['int', 'num', 'pct'].includes(type)

/** A plain number shown by its column type: UGX 4,200 for money, 12% for a percentage. */
export function formatValue(value: number | string | boolean, type: string): string {
  if (typeof value === 'boolean') return value ? 'Yes' : 'No'
  if (typeof value === 'string') return value
  const group = (n: number, digits = 0) => n.toLocaleString('en-US', { minimumFractionDigits: digits, maximumFractionDigits: digits })
  if (isCurrency(type)) return `${type.toUpperCase()} ${group(value, Number.isInteger(value) ? 0 : 2)}`
  if (type === 'pct') return `${group(value * 100, 0)}%`
  if (type === 'int') return group(Math.round(value))
  return group(value, Number.isInteger(value) ? 0 : 2)
}

export type Cell = { ok: true; value: number | string | boolean } | { ok: false; why: 'formula' | 'cycle' | 'ref' | 'text' }

/** Every cell of the sheet, formulas worked out. Rows are numbered from 2 under the header, as A1 notation has it. */
export function computeSheet(sheet: Sheet): Cell[][] {
  const memo = new Map<string, Cell>()
  const visiting = new Set<string>()
  const at = (col: number, row: number): Cell => {
    if (col < 0 || col >= sheet.header.length || row < 2 || row > sheet.rows.length + 1) return { ok: false, why: 'ref' }
    const key = `${col}:${row}`
    const hit = memo.get(key)
    if (hit) return hit
    if (visiting.has(key)) return { ok: false, why: 'cycle' }
    visiting.add(key)
    const result = evaluate(sheet.rows[row - 2][col], sheet.types[col])
    visiting.delete(key)
    memo.set(key, result)
    return result
  }
  const numbersIn = (c1: number, r1: number, c2: number, r2: number): number[] | null => {
    const nums: number[] = []
    for (let r = Math.min(r1, r2); r <= Math.max(r1, r2); r += 1) {
      for (let c = Math.min(c1, c2); c <= Math.max(c1, c2); c += 1) {
        const cell = at(c, r)
        if (!cell.ok) return null
        if (typeof cell.value === 'number') nums.push(cell.value)
      }
    }
    return nums
  }
  const evaluate = (raw: string, type: string): Cell => {
    if (raw.startsWith('=')) {
      const n = formula(raw.slice(1), at, numbersIn)
      return n === null ? { ok: false, why: 'formula' } : { ok: true, value: n }
    }
    if (type === 'text' || type === 'date') return { ok: true, value: raw }
    if (type === 'bool') return { ok: true, value: /^(true|yes|1)$/i.test(raw) }
    if (raw === '') return { ok: true, value: '' }
    const n = Number(raw.replace(/[, ]/g, '').replace(/%$/, ''))
    return Number.isFinite(n) ? { ok: true, value: n } : { ok: true, value: raw }
  }
  return sheet.rows.map((_, r) => sheet.header.map((__, c) => at(c, r + 2)))
}

type At = (col: number, row: number) => Cell
type Numbers = (c1: number, r1: number, c2: number, r2: number) => number[] | null

/** +, -, *, /, brackets, cell references and SUM, MIN, MAX, AVERAGE or COUNT of a range. Anything else is null. */
function formula(src: string, at: At, numbersIn: Numbers): number | null {
  const tokens = src.match(/[A-Z]{1,3}[0-9]{1,7}(?::[A-Z]{1,3}[0-9]{1,7})?|[A-Z]+(?=\()|\d+(?:\.\d+)?|[()+\-*/,]|\s+/g)
  if (!tokens || tokens.join('') !== src) return null
  const list = tokens.filter((t) => t.trim() !== '')
  let p = 0
  const ref = (t: string) => /^([A-Z]{1,3})([0-9]{1,7})$/.exec(t)
  const expr = (): number | null => {
    let v = term()
    while (v !== null && (list[p] === '+' || list[p] === '-')) {
      const op = list[p++]
      const r = term()
      v = r === null ? null : op === '+' ? v + r : v - r
    }
    return v
  }
  const term = (): number | null => {
    let v = unary()
    while (v !== null && (list[p] === '*' || list[p] === '/')) {
      const op = list[p++]
      const r = unary()
      v = r === null || (op === '/' && r === 0) ? null : op === '*' ? v * r : v / r
    }
    return v
  }
  const unary = (): number | null => {
    if (list[p] === '-') {
      p += 1
      const v = unary()
      return v === null ? null : -v
    }
    return atom()
  }
  const atom = (): number | null => {
    const t = list[p++]
    if (t === undefined) return null
    if (/^\d/.test(t)) return Number(t)
    if (t === '(') {
      const v = expr()
      return list[p++] === ')' ? v : null
    }
    if (/^[A-Z]+$/.test(t) && list[p] === '(') {
      p += 1
      const range = /^([A-Z]{1,3})([0-9]{1,7}):([A-Z]{1,3})([0-9]{1,7})$/.exec(list[p++] ?? '')
      if (!range || list[p++] !== ')') return null
      const nums = numbersIn(colIndex(range[1]), Number(range[2]), colIndex(range[3]), Number(range[4]))
      if (!nums) return null
      switch (t) {
        case 'SUM': return nums.reduce((a, b) => a + b, 0)
        case 'COUNT': return nums.length
        case 'MIN': return nums.length ? Math.min(...nums) : null
        case 'MAX': return nums.length ? Math.max(...nums) : null
        case 'AVERAGE': return nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : null
        default: return null
      }
    }
    const r = ref(t)
    if (!r) return null
    const cell = at(colIndex(r[1]), Number(r[2]))
    return cell.ok && typeof cell.value === 'number' ? cell.value : null
  }
  const v = expr()
  return p === list.length ? v : null
}

export interface Range {
  c1: number
  r1: number
  c2: number
  r2: number
  /** The sheet named in front of the range, or null for the note's first. */
  sheet: string | null
}

/** `B2:B7`, `[[#Stock]]!B2:B7`: a range in this note. A reference into another note is null. */
export function parseRange(from: string): Range | null {
  const m = /^(?:\[\[([^\]]*)\]\]!)?([A-Z]{1,3})([0-9]{1,7})(?::([A-Z]{1,3})([0-9]{1,7}))?$/.exec(from.trim())
  if (!m) return null
  let sheet: string | null = null
  if (m[1] !== undefined) {
    // A target with a note name points at another note; one that is only "#block" stays in this one.
    if (!m[1].startsWith('#')) return null
    sheet = m[1].slice(1)
  }
  const c1 = colIndex(m[2])
  const r1 = Number(m[3])
  return { c1, r1, c2: m[4] ? colIndex(m[4]) : c1, r2: m[5] ? Number(m[5]) : r1, sheet }
}

export type Reading = { ok: true; numbers: number[]; labels: string[] } | { ok: false; why: 'other-note' | 'no-sheet' | 'text' }

/** The numbers (and the first-column labels) in a range of one of the note's sheets. */
export function readRange(sheets: Sheet[], from: string): Reading {
  const range = parseRange(from)
  if (!range) return { ok: false, why: 'other-note' }
  const sheet = range.sheet ? sheets.find((s) => s.name === range.sheet || s.id === range.sheet) : sheets[0]
  if (!sheet) return { ok: false, why: 'no-sheet' }
  const cells = computeSheet(sheet)
  const numbers: number[] = []
  const labels: string[] = []
  for (let r = Math.min(range.r1, range.r2); r <= Math.max(range.r1, range.r2); r += 1) {
    for (let c = Math.min(range.c1, range.c2); c <= Math.max(range.c1, range.c2); c += 1) {
      const cell = cells[r - 2]?.[c]
      if (cell?.ok && typeof cell.value === 'number') {
        numbers.push(cell.value)
        labels.push(String(cells[r - 2]?.[0]?.ok ? (cells[r - 2][0] as { value: unknown }).value : r))
      }
    }
  }
  return numbers.length ? { ok: true, numbers, labels } : { ok: false, why: 'text' }
}

/** The result of a tile's function over its numbers; null for an unknown function or no numbers. */
export function tileValue(fn: string, numbers: number[]): number | null {
  if (numbers.length === 0) return null
  const sum = numbers.reduce((a, b) => a + b, 0)
  switch (fn) {
    case 'sum': return sum
    case 'count': return numbers.length
    case 'avg': return sum / numbers.length
    case 'min': return Math.min(...numbers)
    case 'max': return Math.max(...numbers)
    case 'median': {
      const s = [...numbers].sort((a, b) => a - b)
      const mid = Math.floor(s.length / 2)
      return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2
    }
    default: return null
  }
}

export type PlotReading = { ok: true; labels: string[]; values: number[] } | { ok: false; why: 'other-note' | 'no-sheet' | 'columns' | 'text' }

/** A plot's data: its range (header row first), the label column `x` and the value column `y`, by header name. */
export function readPlot(sheets: Sheet[], from: string, x: string, y: string): PlotReading {
  const range = parseRange(from)
  if (!range) return { ok: false, why: 'other-note' }
  const sheet = range.sheet ? sheets.find((s) => s.name === range.sheet || s.id === range.sheet) : sheets[0]
  if (!sheet) return { ok: false, why: 'no-sheet' }
  const lo = Math.min(range.c1, range.c2)
  const hi = Math.max(range.c1, range.c2)
  const cols = sheet.header.map((h, i) => ({ h, i })).filter(({ i }) => i >= lo && i <= hi)
  const xi = cols.find((c) => c.h === x)?.i
  const yi = cols.find((c) => c.h === y)?.i
  if (xi === undefined || yi === undefined) return { ok: false, why: 'columns' }
  const cells = computeSheet(sheet)
  const last = Math.max(range.r1, range.r2)
  const labels: string[] = []
  const values: number[] = []
  // Row 1 is the header, so data starts at row 2.
  for (let r = Math.max(2, Math.min(range.r1, range.r2) + (Math.min(range.r1, range.r2) <= 1 ? 1 : 0)); r <= last; r += 1) {
    const label = cells[r - 2]?.[xi]
    const value = cells[r - 2]?.[yi]
    if (value?.ok && typeof value.value === 'number') {
      labels.push(label?.ok ? String(label.value) : String(r))
      values.push(value.value)
    }
  }
  return values.length ? { ok: true, labels, values } : { ok: false, why: 'text' }
}
