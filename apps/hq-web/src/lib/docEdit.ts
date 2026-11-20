// Doc helpers: the outline of a note's headings, find and replace inside the note's text, a table and an
// image written as markdown, and the one rule that decides whether a block edit may go ahead. Pure.
import { splitFrontmatter } from './hqmd'

export interface Heading {
  level: number
  text: string
  /** Position among all the headings, which is also its position among the headings the viewer draws. */
  n: number
}

/** The note's headings, outside code, in order. A heading is `#` to `######` and a space. */
export function outline(text: string): Heading[] {
  const { body } = splitFrontmatter(text)
  const out: Heading[] = []
  let fenced = false
  for (const line of body.split(/\r?\n/)) {
    if (/^(`{3,}|~{3,})/.test(line)) fenced = !fenced
    const m = !fenced ? /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line) : null
    if (m) out.push({ level: m[1].length, text: m[2].replace(/[*_`]/g, ''), n: out.length })
  }
  return out
}

export interface FindOptions {
  caseSensitive?: boolean
}

function matcher(query: string, o: FindOptions): RegExp | null {
  if (query === '') return null
  return new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), o.caseSensitive ? 'g' : 'gi')
}

/** How many times `query` is in the note's body (the frontmatter is not searched). */
export function countMatches(text: string, query: string, o: FindOptions = {}): number {
  const re = matcher(query, o)
  return re ? (splitFrontmatter(text).body.match(re) ?? []).length : 0
}

/** Replaces every match in the body, leaving the frontmatter and its line endings as they were. Null when nothing matches. */
export function replaceAll(text: string, query: string, by: string, o: FindOptions = {}): string | null {
  const re = matcher(query, o)
  if (!re) return null
  const { body } = splitFrontmatter(text)
  if (!re.test(body)) return null
  const head = text.slice(0, text.length - body.length)
  re.lastIndex = 0
  return head + body.replace(re, () => by)
}

const cell = (s: string) => s.replace(/\|/g, '\\|')

/** A markdown table of `cols` by `rows` (header excluded) with placeholder headers and empty cells. */
export function tableMarkdown(cols: number, rows: number, headers: string[] = []): string {
  const c = Math.min(Math.max(1, Math.floor(cols)), 12)
  const r = Math.min(Math.max(1, Math.floor(rows)), 50)
  const head = Array.from({ length: c }, (_, i) => cell(headers[i]?.trim() || `Column ${i + 1}`))
  const line = (cells: string[]) => `| ${cells.join(' | ')} |`
  return [line(head), line(head.map(() => '---')), ...Array.from({ length: r }, () => line(head.map(() => '')))].join('\n')
}

/** Adds a block at the end of the note, one blank line away from what is there. */
export function appendBlock(text: string, block: string): string {
  const eol = text.includes('\r\n') ? '\r\n' : '\n'
  const body = text.replace(/\s+$/, '')
  return `${body}${eol}${eol}${block.replace(/\n/g, eol)}${eol}`
}

const IMAGE = /\.(png|jpe?g|gif|webp|avif|svg)$/i
export const isImagePath = (p: string) => IMAGE.test(p)

/** The path of `target` as seen from the folder of `from`, `../` where it has to climb. */
export function relativePath(from: string, target: string): string {
  const a = from.split('/').slice(0, -1)
  const b = target.split('/')
  let i = 0
  while (i < a.length && i < b.length - 1 && a[i] === b[i]) i += 1
  return [...Array(a.length - i).fill('..'), ...b.slice(i)].join('/')
}

/** `![alt](path)` with the characters that would end it escaped. */
export function imageMarkdown(alt: string, path: string): string {
  const safe = path.split('/').map(encodeURIComponent).join('/')
  return `![${alt.replace(/[[\]]/g, '')}](${safe})`
}

/** One block's markdown must stay one block: blank-line separated text would be two. */
export function isOneBlock(markdown: string): boolean {
  const fenceCount = (markdown.match(/^(`{3,}|~{3,})/gm) ?? []).length
  if (fenceCount % 2 === 1) return false
  if (fenceCount > 0) return true
  return !/\n[ \t]*\n/.test(markdown.trim())
}
