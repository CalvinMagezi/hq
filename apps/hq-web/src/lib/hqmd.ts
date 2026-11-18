// A reader for hqmd, the note format of the HQ Suite (docs/suite/HQMD.md): ordinary markdown with typed
// fenced blocks (sheet, tile, plot) and a few flat frontmatter keys. This reads enough to show a note;
// it never writes one, and a block it does not know stays what it is: a code block.

export type SuiteKind = 'doc' | 'sheet' | 'slides' | 'dashboard' | 'drive-item'

export interface TypedBlock {
  type: 'sheet' | 'tile' | 'plot'
  attrs: Record<string, string>
  /** The text between the fences. */
  body: string
  /** Line numbers (from 0) of the opening and closing fence in the text this was read from. */
  open?: number
  close?: number
}

export interface Slide {
  title: string
  /** The card's markdown, without its heading and layout comment. */
  body: string
  layout: string | null
}

export interface HqmdNote {
  kind: SuiteKind | null
  fidelity: string | null
  /** The level 1 heading, when the note has one. */
  title: string | null
  blocks: TypedBlock[]
  slides: Slide[]
  /** The note without its frontmatter. */
  body: string
}

const KINDS: readonly string[] = ['doc', 'sheet', 'slides', 'dashboard', 'drive-item']
const TYPED = ['sheet', 'tile', 'plot'] as const

/** The frontmatter as flat key and value pairs, and the text after it. */
export function splitFrontmatter(text: string): { meta: Record<string, string>; body: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text)
  if (!m) return { meta: {}, body: text }
  const meta: Record<string, string> = {}
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line)
    if (kv) meta[kv[1]] = kv[2].trim().replace(/^["']|["']$/g, '')
  }
  return { meta, body: text.slice(m[0].length) }
}

/** `sheet name=Stock types="text,int"` as its type and attributes; null for any other fence. */
export function parseHeader(info: string): { type: TypedBlock['type']; attrs: Record<string, string> } | null {
  const word = /^([a-z][a-z0-9-]*)(?:\s+(.*))?$/.exec(info.trim())
  if (!word || !(TYPED as readonly string[]).includes(word[1])) return null
  const attrs: Record<string, string> = {}
  const re = /([a-z][a-z0-9_-]*)=(?:"((?:[^"\\]|\\.)*)"|([^\s"]+))/g
  for (let a = re.exec(word[2] ?? ''); a; a = re.exec(word[2] ?? '')) attrs[a[1]] = (a[2] ?? a[3]).replace(/\\(["\\])/g, '$1')
  return { type: word[1] as TypedBlock['type'], attrs }
}

/** The typed blocks at the top level of the note, in order. A fence indented or inside a list is plain code. */
export function typedBlocks(body: string): TypedBlock[] {
  const lines = body.split(/\r?\n/)
  const out: TypedBlock[] = []
  for (let i = 0; i < lines.length; i += 1) {
    const open = /^(`{3,}|~{3,})(.*)$/.exec(lines[i])
    if (!open) continue
    const fence = open[1]
    let end = i + 1
    while (end < lines.length && !(lines[end].startsWith(fence) && lines[end].trim() === fence[0].repeat(lines[end].trim().length) && lines[end].trim().length >= fence.length)) end += 1
    const header = parseHeader(open[2])
    if (header) out.push({ ...header, body: lines.slice(i + 1, end).join('\n'), open: i, close: end })
    i = end
  }
  return out
}

/** A slides note: each level 2 heading starts a card; `<!-- layout: x -->` under it names a layout. */
export function slidesOf(body: string): Slide[] {
  const slides: Slide[] = []
  let current: { title: string; lines: string[] } | null = null
  let fenced = false
  for (const line of body.split(/\r?\n/)) {
    if (/^(`{3,}|~{3,})/.test(line)) fenced = !fenced
    const h2 = !fenced ? /^##\s+(.+?)\s*#*\s*$/.exec(line) : null
    if (h2) {
      if (current) slides.push(card(current))
      current = { title: h2[1], lines: [] }
    } else if (current) current.lines.push(line)
  }
  if (current) slides.push(card(current))
  return slides
}

function card(c: { title: string; lines: string[] }): Slide {
  let layout: string | null = null
  const kept = c.lines.filter((l) => {
    const m = /^<!--\s*layout:\s*([a-z0-9-]+)\s*-->$/.exec(l.trim())
    if (m && layout === null) {
      layout = m[1]
      return false
    }
    return true
  })
  return { title: c.title, body: kept.join('\n').trim(), layout }
}

export function readHqmd(text: string): HqmdNote {
  const { meta, body } = splitFrontmatter(text)
  const kind = KINDS.includes(meta.suite_kind) ? (meta.suite_kind as SuiteKind) : null
  const h1 = /^#\s+(.+?)\s*#*\s*$/m.exec(body)
  return { kind, fidelity: meta.suite_fidelity ?? null, title: h1?.[1] ?? null, blocks: typedBlocks(body), slides: kind === 'slides' ? slidesOf(body) : [], body }
}

/** Whether the pane and the Vault page draw this note with a suite viewer instead of as plain markdown. */
export function hasViewer(note: HqmdNote): boolean {
  return note.kind === 'sheet' || note.kind === 'slides' || note.kind === 'dashboard'
}
