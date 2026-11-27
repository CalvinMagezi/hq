import type { TokenizerAndRendererExtension, Tokens } from 'marked'

/**
 * `[[Note]]`, `[[Note#Heading|label]]`, `[[Folder/Note]]` and `![[embed]]`, read the way the
 * server reads them (`crates/hq-vault/src/links.rs`). The shared corpus
 * `crates/hq-vault/tests/fixtures/links_cases.json` pins both sides to the same answers.
 *
 * It is a marked extension, not a text replace, so a link inside a code block or a code span
 * is left as the code it is.
 */
export interface Wikilink {
  /** The note's name or path, without a heading, block id or label. Never empty. */
  target: string
  heading: string | null
  block: string | null
  /** The label after `|`. */
  alias: string | null
  /** `![[x]]`: shown in place on the web only for images; a note embed is a link here. */
  embed: boolean
  /** The part before `|`, as written, for showing when there is no label. */
  written: string
}

const MAX_INNER = 400

/**
 * What is between `[[` and `]]`, or null when it is not a link to another note: nothing there,
 * or only a heading of this very note (`[[#Goals]]`).
 */
export function parseWikilink(inner: string, embed = false): Wikilink | null {
  const pipe = inner.indexOf('|')
  const label = pipe >= 0 ? inner.slice(pipe + 1).trim() : ''
  // In a table a pipe is written `\|`.
  const left = (pipe >= 0 ? inner.slice(0, pipe) : inner).replace(/\\+$/, '')
  const hash = left.indexOf('#')
  const target = (hash >= 0 ? left.slice(0, hash) : left).trim()
  if (!target) return null
  const frag = hash >= 0 ? left.slice(hash + 1).trim() : ''
  const isBlock = frag.startsWith('^') && frag.length > 1
  return {
    target,
    heading: frag && !isBlock ? frag : null,
    block: isBlock ? frag.slice(1) : null,
    alias: label || null,
    embed,
    written: left.trim(),
  }
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

export interface WikilinkToken extends Tokens.Generic {
  type: 'wikilink'
  link: Wikilink
}

/** The button a wikilink draws: the note to open is its `data-vault-note`. */
export function wikilinkHtml(link: Wikilink): string {
  const label = link.alias ?? link.written
  return `<button type="button" class="md-wikilink a-link" data-vault-note="${escapeHtml(link.target)}" aria-label="Open note ${escapeHtml(label)}">${escapeHtml(label)}</button>`
}

/** Register with `marked.use({ extensions: [wikilinkExtension] })`. */
export const wikilinkExtension: TokenizerAndRendererExtension = {
  name: 'wikilink',
  level: 'inline',
  start(src: string) {
    const at = src.search(/!?\[\[/)
    return at < 0 ? undefined : at
  },
  tokenizer(src: string) {
    const m = /^(!?)\[\[([^[\]\n]+)\]\]/.exec(src)
    if (!m || m[2].length > MAX_INNER) return undefined
    const link = parseWikilink(m[2], m[1] === '!')
    if (!link) return undefined
    return { type: 'wikilink', raw: m[0], link } satisfies WikilinkToken
  },
  renderer(token) {
    return wikilinkHtml((token as WikilinkToken).link)
  },
}
