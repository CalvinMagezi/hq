// Review comments as the viewers show them (docs/suite/HQMD.md, `comment`): how many are open, which block a
// new comment goes on, and how the reading viewer keeps them out of the page. HQ does the writing; this reads.
import type { SuiteBlock, SuiteComment } from './suiteApi'
import { typedBlocks } from './hqmd'

export interface CommentCounts {
  open: number
  resolved: number
}

/** The comments in a note's text, counted from the blocks themselves, so the count needs no call and works offline. */
export function countComments(text: string): CommentCounts {
  const out: CommentCounts = { open: 0, resolved: 0 }
  for (const b of typedBlocks(text)) {
    if (b.type !== 'comment') continue
    if (b.attrs.state === 'resolved') out.resolved += 1
    else out.open += 1
  }
  return out
}

/** Open comments first, in the order they were written; resolved ones after, for a collapsed list. */
export function splitComments<C extends SuiteComment>(list: C[]): { open: C[]; resolved: C[] } {
  return { open: list.filter((c) => c.state !== 'resolved'), resolved: list.filter((c) => c.state === 'resolved') }
}

/** Where the person was when they asked for the comments: a kind of block (the nth sheet), or text in it. */
export interface AnchorHint {
  kind?: string
  nth?: number
  text?: string
}

/** The blocks a comment can be about: every block but a comment. */
export const commentable = (blocks: SuiteBlock[]) => blocks.filter((b) => b.kind !== 'comment')

const squash = (s: string) => s.replace(/\s+/g, ' ').trim()

/**
 * The block the hint points at, or null. Text wins (a block that is exactly it, then the first that holds it),
 * then the nth block of a kind. With no hint, or none that matches, the caller offers its own default.
 */
export function pickAnchor(blocks: SuiteBlock[], hint?: AnchorHint): SuiteBlock | null {
  const list = commentable(blocks)
  const text = hint?.text ? squash(hint.text) : ''
  if (text) {
    const exact = list.find((b) => squash(b.src) === text)
    if (exact) return exact
    const within = list.find((b) => squash(b.src).includes(text))
    if (within) return within
  }
  if (hint?.kind) {
    const same = list.filter((b) => b.kind === hint.kind)
    return same[hint.nth ?? 0] ?? null
  }
  return null
}

/** A block as one line in a list of choices: its kind and its first words. */
export function blockChoice(b: SuiteBlock, max = 48): { kind: string; first: string } {
  const first = squash(b.src.split('\n').find((l) => l.trim() !== '') ?? '')
  return { kind: b.kind, first: first.length > max ? `${first.slice(0, max - 1)}…` : first }
}

/** What `marked` hands the renderer: only what is read here. */
interface Tok {
  type: string
  lang?: string
  raw?: string
  text?: string
  [key: string]: unknown
}

/** marked hands over the whole info string as `lang`, attributes and all. */
export const isCommentLang = (lang: string | undefined) => (lang ?? '').trim().split(/\s+/, 1)[0] === 'comment'
const isComment = (tok: Tok) => tok.type === 'code' && isCommentLang(tok.lang)

/**
 * The reading viewers never draw a comment block as code. Each run of them (the ones after one block, with only
 * blank space between) becomes the first token of the run, carrying how many there are and how many are open, so
 * the renderer can leave one small marker after the block they are about.
 */
export function foldComments<T extends Tok>(tokens: T[]): T[] {
  const out: T[] = []
  for (let i = 0; i < tokens.length; i += 1) {
    const tok = tokens[i]
    if (!isComment(tok)) {
      out.push(tok)
      continue
    }
    let open = 0
    let total = 0
    let j = i
    while (j < tokens.length && (isComment(tokens[j]) || tokens[j].type === 'space')) {
      if (isComment(tokens[j])) {
        total += 1
        if (!/\bstate=resolved\b/.test(firstLine(tokens[j]))) open += 1
      }
      j += 1
    }
    out.push({ ...tok, hqComments: total, hqOpen: open })
    i = j - 1
  }
  return out
}

const firstLine = (tok: Tok) => (tok.raw ?? '').split('\n', 1)[0]

/** The small marker left where a run of comments was. Its words come from the caller; this only builds the element. */
export function commentMarker(open: number, label: string): string {
  const safe = label.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
  return `<div class="md-cmt-mark${open > 0 ? ' is-open' : ''}" role="note">${safe}</div>`
}
