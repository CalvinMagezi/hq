// A dashboard's layout: the order of its tiles and plots, and how much room each takes. The order is the order of
// the blocks in the note; the room is a `size=small|wide|full` attribute on the block's fence, a hint for this viewer
// (docs/suite/HQMD.md, "Dashboard layout"). Each function takes the whole note and returns the whole note with one
// place changed, or null when the block is not the one the caller saw. Every other byte comes back as it was.
import { typedBlocks, type TypedBlock } from './hqmd'

export type Size = 'small' | 'wide' | 'full'
export const SIZES: readonly Size[] = ['small', 'wide', 'full']

/** A tile is a small number, a plot is wide enough to read: what a block takes when it names no size. */
export const defaultSize = (type: TypedBlock['type']): Size => (type === 'tile' ? 'small' : 'wide')

/** The size a block asks for; one the viewer does not know counts as no size. */
export function sizeOf(block: TypedBlock): Size {
  const asked = block.attrs.size
  return SIZES.find((s) => s === asked) ?? defaultSize(block.type)
}

/** The tiles and plots of a note, in the order they are written. */
export const layoutBlocks = (text: string): TypedBlock[] => typedBlocks(text).filter((b) => b.type === 'tile' || b.type === 'plot')

const ATTR = /([a-z][a-z0-9_-]*)=(?:"(?:[^"\\]|\\.)*"|[^\s"]+)/g

function split(text: string): { rows: string[]; eol: string } {
  return { rows: text.split(/\r?\n/), eol: text.includes('\r\n') ? '\r\n' : '\n' }
}

/** The block at `n` if it is still the one whose opening line is `head`, and closed. */
function find(text: string, n: number, head: string | undefined): TypedBlock | null {
  const block = layoutBlocks(text)[n]
  if (!block || block.open === undefined || block.close === undefined || head === undefined || block.head !== head) return null
  return block.close < split(text).rows.length ? block : null
}

/** Moves the block at layout position `from` to position `to`; the text between the blocks stays where it is. */
export function moveBlock(text: string, from: number, to: number, head: string | undefined): string | null {
  const blocks = layoutBlocks(text)
  if (from < 0 || to < 0 || from >= blocks.length || to >= blocks.length) return null
  if (!find(text, from, head)) return null
  if (from === to) return text
  const { rows, eol } = split(text)
  if (blocks.some((b) => b.close === undefined || b.close >= rows.length)) return null
  const bodies = blocks.map((b) => rows.slice(b.open as number, (b.close as number) + 1))
  const order = blocks.map((_, i) => i)
  order.splice(to, 0, ...order.splice(from, 1))
  const out: string[] = []
  let cursor = 0
  blocks.forEach((b, k) => {
    out.push(...rows.slice(cursor, b.open as number), ...bodies[order[k]])
    cursor = (b.close as number) + 1
  })
  out.push(...rows.slice(cursor))
  return out.join(eol)
}

/**
 * Sets the size of the block at layout position `n`. The default size is written as no attribute at all, so a
 * note that was never arranged stays as it was. The other attributes of the fence are kept as written.
 */
export function setSize(text: string, n: number, head: string | undefined, size: Size): string | null {
  const block = find(text, n, head)
  if (!block) return null
  const open = block.open as number
  const { rows, eol } = split(text)
  const line = rows[open]
  const keep = size === defaultSize(block.type) ? null : size
  // Find the attribute by walking the same pairs the reader does, so a `size=` inside a quoted label is left alone.
  const info = /^(`{3,}|~{3,})(\S*)/.exec(line)
  if (!info) return null
  const from = info[0].length
  let found: { at: number; len: number } | null = null
  ATTR.lastIndex = from
  for (let m = ATTR.exec(line); m; m = ATTR.exec(line)) {
    if (m[1] === 'size') found = { at: m.index, len: m[0].length }
  }
  let next: string
  if (found) {
    next = keep ? `${line.slice(0, found.at)}size=${keep}${line.slice(found.at + found.len)}` : `${line.slice(0, found.at).replace(/\s+$/, '')}${line.slice(found.at + found.len)}`
  } else if (keep) {
    next = `${line.replace(/\s+$/, '')} size=${keep}`
  } else {
    return text
  }
  if (next === line) return text
  rows[open] = next
  return rows.join(eol)
}
