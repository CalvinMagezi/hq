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

export interface CardMeta {
  layout: string | null
  /** `fade`, or `push slow`: the name and an optional speed. */
  transition: string | null
  background: string | null
  color: string | null
  notes: string | null
}

export interface Slide extends CardMeta {
  title: string
  /** The card's markdown, without its heading and its layout, transition, colour and notes comments. */
  body: string
}

export interface HqmdNote {
  kind: SuiteKind | null
  fidelity: string | null
  /** The level 1 heading, when the note has one. */
  title: string | null
  blocks: TypedBlock[]
  slides: Slide[]
  /** The deck's own transition and colours, for a slides note. */
  deck: CardMeta | null
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

const META_KEYS = ['layout', 'transition', 'background', 'color', 'notes'] as const
export type MetaKey = (typeof META_KEYS)[number]

/** A card's comment lines (outside code) taken out: where each one sits (line, count of lines), and the rest. */
export function cardComments(lines: string[]): { spans: { key: MetaKey; start: number; end: number; value: string }[] } {
  const spans: { key: MetaKey; start: number; end: number; value: string }[] = []
  let fenced = false
  for (let i = 0; i < lines.length; i += 1) {
    if (/^(`{3,}|~{3,})/.test(lines[i])) fenced = !fenced
    if (fenced) continue
    const m = /^<!--\s*(layout|transition|background|color|notes):\s?([\s\S]*)$/.exec(lines[i].trim())
    if (!m) continue
    let end = i
    let text = m[2]
    while (!text.includes('-->') && end + 1 < lines.length) {
      end += 1
      text += `\n${lines[end]}`
    }
    if (!text.includes('-->')) continue
    const value = text.slice(0, text.indexOf('-->')).trim()
    if (text.slice(text.indexOf('-->') + 3).trim() !== '') continue
    spans.push({ key: m[1] as MetaKey, start: i, end, value })
    i = end
  }
  return { spans }
}

/** Splits a card's lines into its meta comments and the rest. The first comment of a key wins, as the pptx writer reads it. */
export function splitCard(lines: string[]): { meta: CardMeta; rest: string[] } {
  const { spans } = cardComments(lines)
  const meta: CardMeta = { layout: null, transition: null, background: null, color: null, notes: null }
  const drop = new Set<number>()
  for (const s of spans) {
    if (meta[s.key] === null) meta[s.key] = s.value
    for (let i = s.start; i <= s.end; i += 1) drop.add(i)
  }
  return { meta, rest: lines.filter((_, i) => !drop.has(i)) }
}

function card(c: { title: string; lines: string[] }): Slide {
  const { meta, rest } = splitCard(c.lines)
  return { title: c.title, body: rest.join('\n').trim(), ...meta }
}

/** The deck's own transition and colours: comments before the first card. */
export function deckMeta(body: string): CardMeta {
  const lines = body.split(/\r?\n/)
  const first = lines.findIndex((l) => /^##\s+\S/.test(l))
  return splitCard(first < 0 ? lines : lines.slice(0, first)).meta
}

export function readHqmd(text: string): HqmdNote {
  const { meta, body } = splitFrontmatter(text)
  const kind = KINDS.includes(meta.suite_kind) ? (meta.suite_kind as SuiteKind) : null
  const h1 = /^#\s+(.+?)\s*#*\s*$/m.exec(body)
  return { kind, fidelity: meta.suite_fidelity ?? null, title: h1?.[1] ?? null, blocks: typedBlocks(body), slides: kind === 'slides' ? slidesOf(body) : [], deck: kind === 'slides' ? deckMeta(body) : null, body }
}

/** Whether the pane and the Vault page draw this note with a suite viewer instead of as plain markdown. */
export function hasViewer(note: HqmdNote): boolean {
  return note.kind === 'sheet' || note.kind === 'slides' || note.kind === 'dashboard'
}
