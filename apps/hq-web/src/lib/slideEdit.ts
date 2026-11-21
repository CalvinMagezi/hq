// Card-level edits to a slides note, as text patches like hqmdEdit: add, copy, delete and move a card, and set
// a card's layout, transition, colours and speaker notes (the comments the pptx writer reads, docs/suite/SLIDES.md).
// Each takes the whole note and returns it with one thing changed, or null when what the caller saw is gone.
//
// These work on the note text, so they save through the same hash-guarded path as every other edit and keep
// working offline. The planned /api/suite/slides/* routes (lib/suiteOps.ts) are not needed for them.
import { cardComments, type MetaKey } from './hqmd'

export const LAYOUTS = ['content', 'two-column'] as const
export const TRANSITIONS = ['none', 'fade', 'push', 'wipe', 'split', 'cover', 'dissolve'] as const
export const SPEEDS = ['slow', 'medium', 'fast'] as const

function split(text: string): { rows: string[]; eol: string } {
  return { rows: text.split(/\r?\n/), eol: text.includes('\r\n') ? '\r\n' : '\n' }
}

/** Where each card sits: its heading line and the line after its last (headings inside code do not count). */
export function cardSpans(rows: string[]): { start: number; end: number }[] {
  const heads: number[] = []
  let fenced = false
  rows.forEach((l, i) => {
    if (/^(`{3,}|~{3,})/.test(l)) fenced = !fenced
    if (!fenced && /^##\s+\S/.test(l)) heads.push(i)
  })
  return heads.map((start, i) => ({ start, end: heads[i + 1] ?? rows.length }))
}

const trimEnd = (lines: string[]) => {
  const out = [...lines]
  while (out.length && out[out.length - 1].trim() === '') out.pop()
  return out
}

/** Puts the cards back in the order given, one blank line between, the lines above the first card untouched. */
function join(rows: string[], spans: { start: number; end: number }[], order: string[][]): string[] {
  const head = spans.length ? rows.slice(0, spans[0].start) : rows
  const endsWithBlank = rows.length > 0 && rows[rows.length - 1] === ''
  const out = [...head]
  order.forEach((card, i) => {
    out.push(...trimEnd(card))
    if (i < order.length - 1) out.push('')
  })
  if (endsWithBlank || order.length) out.push('')
  return out
}

const cardsOf = (rows: string[], spans: { start: number; end: number }[]) => spans.map((s) => rows.slice(s.start, s.end))

/** A new card after the nth card (-1 puts it first); the first card of an empty deck goes at the end of the note. */
export function addCard(text: string, after: number, title: string): string | null {
  const { rows, eol } = split(text)
  const spans = cardSpans(rows)
  if (after < -1 || after >= spans.length) return null
  const cards = cardsOf(rows, spans)
  cards.splice(after + 1, 0, [`## ${title.replace(/\r?\n/g, ' ').trim() || 'New card'}`, '', ''])
  if (spans.length === 0) return `${trimEnd(rows).join(eol)}${eol}${eol}${cards[0][0]}${eol}`
  return join(rows, spans, cards).join(eol)
}

export function duplicateCard(text: string, n: number): string | null {
  const { rows, eol } = split(text)
  const spans = cardSpans(rows)
  if (n < 0 || n >= spans.length) return null
  const cards = cardsOf(rows, spans)
  cards.splice(n + 1, 0, [...cards[n]])
  return join(rows, spans, cards).join(eol)
}

/** Removes the nth card; `title` is what the caller saw, so a card that moved is not removed by mistake. */
export function deleteCard(text: string, n: number, title: string): string | null {
  const { rows, eol } = split(text)
  const spans = cardSpans(rows)
  if (n < 0 || n >= spans.length || rows[spans[n].start].replace(/^##\s+/, '').replace(/\s*#*\s*$/, '') !== title) return null
  const cards = cardsOf(rows, spans)
  cards.splice(n, 1)
  return join(rows, spans, cards).join(eol)
}

export function moveCard(text: string, from: number, to: number): string | null {
  const { rows, eol } = split(text)
  const spans = cardSpans(rows)
  if (from < 0 || to < 0 || from >= spans.length || to >= spans.length) return null
  if (from === to) return text
  const cards = cardsOf(rows, spans)
  const [one] = cards.splice(from, 1)
  cards.splice(to, 0, one)
  return join(rows, spans, cards).join(eol)
}

const clean = (v: string) => v.replace(/--+>/g, '').replace(/<!--/g, '').trim()

/** Sets one of a card's comments (layout, transition, background, color, notes), or removes it when `value` is null. */
export function setCardMeta(text: string, n: number, key: MetaKey, value: string | null): string | null {
  const { rows, eol } = split(text)
  const spans = cardSpans(rows)
  if (n < 0 || n >= spans.length) return null
  const { start, end } = spans[n]
  const inner = rows.slice(start + 1, end)
  const found = cardComments(inner).spans.find((s) => s.key === key)
  const v = value === null ? '' : clean(value)
  const line = v === '' ? null : `<!-- ${key}: ${v} -->`
  if (found) {
    inner.splice(found.start, found.end - found.start + 1, ...(line ? line.split('\n') : []))
  } else if (line) {
    inner.unshift(...line.split('\n'))
  } else return text
  rows.splice(start + 1, end - start - 1, ...inner)
  return rows.join(eol)
}

// ---- colours: the contrast rule the pptx writer warns about (under 3:1), mirrored ----

export const MIN_CONTRAST = 3
const THEME_BACKGROUND = '#FFFFFF'
const THEME_TEXT = '#000000'

/** `#RGB` or `#RRGGBB` as channels, else null. */
export function parseColor(v: string | null | undefined): [number, number, number] | null {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec((v ?? '').trim())
  if (!m) return null
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1]
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number]
}

const lum = ([r, g, b]: [number, number, number]) => {
  const f = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/** WCAG contrast ratio of two colours, 1 to 21. */
export function contrastRatio(a: [number, number, number], b: [number, number, number]): number {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

/** The ratio when text on its background is hard to read (under 3:1), counting the theme's white and black for what is unset; else null. */
export function contrastWarning(color: string | null, background: string | null): number | null {
  const fg = parseColor(color ?? THEME_TEXT)
  const bg = parseColor(background ?? THEME_BACKGROUND)
  if (!fg || !bg) return null
  if (color === null && background === null) return null
  const r = contrastRatio(fg, bg)
  return r < MIN_CONTRAST ? Math.round(r * 10) / 10 : null
}

/** `push slow` as its name and speed; null for a name the writer would not use. */
export function parseTransition(v: string | null): { name: string; speed: string | null } | null {
  const [name, speed] = (v ?? '').trim().split(/\s+/)
  if (!name || !(TRANSITIONS as readonly string[]).includes(name)) return null
  return { name, speed: speed && (SPEEDS as readonly string[]).includes(speed) ? speed : null }
}
