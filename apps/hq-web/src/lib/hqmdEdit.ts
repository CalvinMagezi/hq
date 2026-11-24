// Edits to an hqmd note, made the way the format asks: small, by block, and refused when stale. Each function
// takes the whole note and returns the whole note with one place changed, or null when it would not be safe
// (the thing it names is not what the caller saw). Every other line comes back as it was, line endings included.
import { cardComments, splitFrontmatter, typedBlocks } from './hqmd'
import { readSheet } from './sheetCalc'

/** The text split into lines, and the line ending to join them with again. */
function lines(text: string): { rows: string[]; eol: string } {
  return { rows: text.split(/\r?\n/), eol: text.includes('\r\n') ? '\r\n' : '\n' }
}

function cellText(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
}

function splitCells(line: string): string[] {
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

const row = (cells: string[]) => `| ${cells.map(cellText).join(' | ')} |`

/** The kind of value a column holds decides what may be typed into it. */
export function acceptCell(raw: string, type: string): string | null {
  const v = raw.trim()
  if (v.startsWith('=')) return v
  if (v === '') return ''
  if (type === 'text') return v
  if (type === 'date') return /^\d{4}-\d{2}-\d{2}(T[\d:.+Z-]*)?$/.test(v) ? v : null
  if (type === 'bool') return /^(true|false|yes|no)$/i.test(v) ? v.toLowerCase() : null
  // Numbers, percentages and money are stored as the plain number, so "4,200" is 4200.
  const n = Number(v.replace(/[, ]/g, '').replace(/^[A-Za-z]{3}(?=\d)/, ''))
  return Number.isFinite(n) ? String(n) : null
}

/**
 * Sets one cell of the nth sheet. `was` is what the caller saw in the cell; if the note holds something else
 * now, the edit is stale and refused.
 */
export function setCell(text: string, sheetIndex: number, r: number, c: number, was: string, value: string): string | null {
  const block = typedBlocks(text).filter((b) => b.type === 'sheet')[sheetIndex]
  const sheet = block && readSheet(block)
  if (!block || !sheet || block.open === undefined || !sheet.rows[r] || c < 0 || c >= sheet.header.length) return null
  if (sheet.rows[r][c] !== was) return null
  const accepted = acceptCell(value, sheet.types[c])
  if (accepted === null) return null
  const { rows, eol } = lines(text)
  // The header and delimiter rows sit above the data, and blank lines are not part of a table.
  const body = rows.map((l, i) => ({ l, i })).filter(({ l, i }) => i > block.open! && i < (block.close ?? rows.length) && l.trim() !== '')
  const target = body[2 + r]
  if (!target) return null
  const cells = splitCells(target.l)
  cells[c] = accepted
  rows[target.i] = row(cells)
  return rows.join(eol)
}

/** A new empty row at the end of the nth sheet. */
export function addRow(text: string, sheetIndex: number): string | null {
  const block = typedBlocks(text).filter((b) => b.type === 'sheet')[sheetIndex]
  const sheet = block && readSheet(block)
  if (!block || !sheet || block.open === undefined || block.close === undefined || sheet.totalRows !== null) return null
  const { rows, eol } = lines(text)
  rows.splice(block.close, 0, row(sheet.header.map(() => '')))
  return rows.join(eol)
}

/** The line of the nth level 2 heading (outside code), or -1. */
function headingLine(rows: string[], n: number): number {
  let fenced = false
  let seen = -1
  for (let i = 0; i < rows.length; i += 1) {
    if (/^(`{3,}|~{3,})/.test(rows[i])) fenced = !fenced
    if (!fenced && /^##\s+\S/.test(rows[i])) {
      seen += 1
      if (seen === n) return i
    }
  }
  return -1
}

/** Renames the nth card. `was` is the title the caller saw. */
export function setSlideTitle(text: string, n: number, was: string, title: string): string | null {
  const { rows, eol } = lines(text)
  const at = headingLine(rows, n)
  const clean = title.replace(/\r?\n/g, ' ').trim()
  if (at < 0 || clean === '' || rows[at].replace(/^##\s+/, '').replace(/\s*#*\s*$/, '') !== was) return null
  rows[at] = `## ${clean}`
  return rows.join(eol)
}

/** Replaces the text of the nth card, keeping its heading and layout comment. `was` is the body the caller saw. */
export function setSlideBody(text: string, n: number, was: string, body: string): string | null {
  const { rows, eol } = lines(text)
  const at = headingLine(rows, n)
  if (at < 0) return null
  let end = rows.length
  for (let i = at + 1; i < rows.length; i += 1) {
    if (/^##\s+\S/.test(rows[i])) {
      end = i
      break
    }
  }
  const inner = rows.slice(at + 1, end)
  const { spans } = cardComments(inner)
  const meta = new Set<number>()
  for (const sp of spans) for (let i = sp.start; i <= sp.end; i += 1) meta.add(i)
  const current = inner.filter((_, i) => !meta.has(i)).join('\n').trim()
  if (current !== was.trim()) return null
  const keep = inner.filter((_, i) => meta.has(i))
  const fresh = body.replace(/\r?\n/g, '\n').trim()
  const trailing = end < rows.length ? [''] : []
  rows.splice(at + 1, end - at - 1, ...keep, '', ...(fresh ? fresh.split('\n') : []), ...trailing)
  return rows.join(eol)
}

/** Whether the note's own header lets the web change it. A view-only file is never edited. */
export function editable(text: string): boolean {
  return splitFrontmatter(text).meta.suite_fidelity !== 'view'
}
