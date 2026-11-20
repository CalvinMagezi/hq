// What a reply read and what it made, taken from the tool calls the chat socket already sends.
// Nothing here asks the server for more: a tool whose arguments name no path or address adds nothing.
import { unwrap } from './stepWords'
import type { ReplyFile } from './replyFiles'

export type SourceKind = 'note' | 'file' | 'web'

export interface ReplySource {
  kind: SourceKind
  /** What the chip says: a note's name, a file's name or a site's name, never a raw address. */
  title: string
  /** The path or address it came from; one source is listed once. */
  ref: string
  /** Still being read. */
  reading: boolean
}

export interface MadeNote {
  title: string
  path: string
}

interface StepLike {
  toolName: string
  status: 'running' | 'done'
  inputArgs?: Record<string, unknown>
}

const NOTE_READS = new Set(['vault_read', 'vault_read_section', 'vault_outline', 'context_packet'])
const FILE_READS = new Set(['read_file', 'convert_to_markdown', 'ocr_extract_text'])
const NOTE_WRITES = new Set(['vault_write_note'])
const TURN_WRITES = new Set([
  'vault_write_note',
  'vault_append_note',
  'vault_patch_section',
  'vault_frontmatter_update',
  'write_file',
  'edit_file',
])

function text(args: Record<string, unknown> | undefined, key: string): string | null {
  const v = args?.[key]
  return typeof v === 'string' && v.trim() ? v.trim() : null
}

function baseName(path: string): string {
  const last = path.split('/').filter(Boolean).pop() ?? path
  return last.replace(/\.md$/, '')
}

const TRACKING_PARAM = /^(utm_|fbclid$|gclid$|mc_|ref$|ref_src$|igshid$|_hs)/i

/** One page, whatever the way it was linked: no fragment, tracking parameters, `www.`, scheme or trailing slash. */
export function canonicalUrl(url: string): string | null {
  try {
    const u = new URL(url)
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null
    const host = u.hostname.toLowerCase().replace(/^www\./, '')
    if (!host) return null
    const kept = [...u.searchParams.entries()].filter(([k]) => !TRACKING_PARAM.test(k)).sort(([a], [b]) => a.localeCompare(b))
    const query = kept.length ? `?${kept.map(([k, v]) => `${k}=${v}`).join('&')}` : ''
    const path = u.pathname.replace(/\/+$/, '')
    return `${host}${path}${query}`
  } catch {
    return null
  }
}

/** The last readable piece of a page's path: `/prices/matooke-2024.html` -> `matooke 2024`. */
function pathTail(url: string): string | null {
  try {
    const last = new URL(url).pathname.split('/').filter(Boolean).pop()
    if (!last) return null
    const tail = decodeURIComponent(last).replace(/\.[a-z0-9]{1,5}$/i, '').replace(/[-_+]+/g, ' ').trim()
    return tail || null
  } catch {
    return null
  }
}

function siteName(url: string): string | null {
  try {
    return new URL(url).hostname.replace(/^www\./, '') || null
  } catch {
    return null
  }
}

/** Every note, file and page the steps read, each once, in the order HQ reached it. */
export function sourcesOf(steps: StepLike[]): ReplySource[] {
  const out: ReplySource[] = []
  const seen = new Map<string, number>()
  for (const step of steps) {
    const [name, args] = unwrap(step.toolName, step.inputArgs)
    let kind: SourceKind | null = null
    let ref: string | null = null
    let title: string | null = null
    if (NOTE_READS.has(name)) {
      ref = text(args, 'path')
      kind = 'note'
      title = ref && baseName(ref)
    } else if (FILE_READS.has(name)) {
      ref = text(args, 'path')
      kind = 'file'
      title = ref && (ref.split('/').filter(Boolean).pop() ?? ref)
    } else if (name === 'web_fetch') {
      ref = text(args, 'url')
      kind = 'web'
      title = ref && siteName(ref)
    }
    if (!kind || !ref || !title) continue
    const reading = step.status === 'running'
    const key = kind === 'web' ? (canonicalUrl(ref) ?? ref) : ref
    const at = seen.get(key)
    if (at !== undefined) {
      out[at] = { ...out[at], reading: out[at].reading || reading }
      continue
    }
    seen.set(key, out.length)
    out.push({ kind, title, ref, reading })
  }
  return labelSamePages(out)
}

/** Two different pages of one site are told apart by the end of their path, not both called by the domain. */
function labelSamePages(list: ReplySource[]): ReplySource[] {
  const perSite = new Map<string, number>()
  for (const s of list) if (s.kind === 'web') perSite.set(s.title, (perSite.get(s.title) ?? 0) + 1)
  return list.map((s) => {
    if (s.kind !== 'web' || (perSite.get(s.title) ?? 0) < 2) return s
    const tail = pathTail(s.ref)
    return tail ? { ...s, title: `${s.title}: ${tail}` } : s
  })
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function writtenPathsOf(steps: StepLike[]): Set<string> {
  const written = new Set<string>()
  for (const step of steps) {
    const [name, args] = unwrap(step.toolName, step.inputArgs)
    if (!TURN_WRITES.has(name)) continue
    const path = text(args, 'path')
    if (path) written.add(path)
  }
  return written
}

function isNoteCited(source: ReplySource, content: string): boolean {
  if (!content) return false
  const lower = content.toLowerCase()
  const refLower = source.ref.toLowerCase()
  const stemLower = refLower.replace(/\.md$/, '')
  if (lower.includes(refLower) || lower.includes(stemLower)) return true
  if (lower.includes(encodeURIComponent(source.ref).toLowerCase())) return true
  const wikilink = new RegExp(`\\[\\[[^\\]]*\\b${escapeRegExp(source.title)}\\b[^\\]]*\\]\\]`, 'i')
  if (wikilink.test(content)) return true
  if (source.title.length >= 2) {
    const titleRe = new RegExp(`(^|[^A-Za-z0-9_])${escapeRegExp(source.title)}([^A-Za-z0-9_]|$)`)
    if (titleRe.test(content)) return true
  }
  return false
}

/** How many source chips show before "+N more". */
export const SOURCE_CAP = 5

/** The chips to show now and how many wait behind "+N more". */
export function splitSources(sources: ReplySource[], expanded: boolean, cap = SOURCE_CAP): { shown: ReplySource[]; hidden: number } {
  if (expanded || sources.length <= cap) return { shown: sources, hidden: 0 }
  return { shown: sources.slice(0, cap), hidden: sources.length - cap }
}

function linksIn(content: string): { keys: Set<string>; hosts: Set<string> } {
  const keys = new Set<string>()
  const hosts = new Set<string>()
  for (const m of content.matchAll(/https?:\/\/[^\s)>\]"'<]+/gi)) {
    const key = canonicalUrl(m[0].replace(/[.,;:!?]+$/, ''))
    if (!key) continue
    keys.add(key)
    hosts.add(key.split(/[/?]/)[0])
  }
  return { keys, hosts }
}

function isCited(source: ReplySource, content: string, links: ReturnType<typeof linksIn>): boolean {
  if (!content) return false
  if (source.kind === 'note') return isNoteCited(source, content)
  if (source.kind === 'file') return content.toLowerCase().includes(source.title.toLowerCase())
  const key = canonicalUrl(source.ref)
  if (!key) return false
  if (links.keys.has(key)) return true
  const host = key.split(/[/?]/)[0]
  // When the reply links some pages of a site, only those pages are cited; a bare mention of the site names all of it.
  return !links.hosts.has(host) && content.toLowerCase().includes(host)
}

/**
 * Sources relevant to an answer: a note only when it is cited or was written this turn.
 * When the reply cites or links any page or file, only those show of the pages and files; otherwise all of them do.
 */
export function relevantSources(steps: StepLike[], content: string): ReplySource[] {
  const all = sourcesOf(steps)
  const written = writtenPathsOf(steps)
  const kept = all.filter((s) => s.kind !== 'note' || written.has(s.ref) || isNoteCited(s, content))
  const links = linksIn(content)
  // Notes already passed their own test above; the preference is for pages and files.
  const outside = kept.filter((s) => s.kind !== 'note')
  const cited = new Set(outside.filter((s) => isCited(s, content, links)))
  return cited.size > 0 ? kept.filter((s) => s.kind === 'note' || cited.has(s)) : kept
}

/** The notes this reply wrote, newest last. A path under the Vault's notes is the only kind the pane can open. */
export function madeNotes(steps: StepLike[]): MadeNote[] {
  const out: MadeNote[] = []
  for (const step of steps) {
    const [name, args] = unwrap(step.toolName, step.inputArgs)
    if (!NOTE_WRITES.has(name)) continue
    const path = text(args, 'path')
    if (!path || !path.startsWith('Notebooks/')) continue
    if (out.some((m) => m.path === path)) continue
    out.push({ title: baseName(path), path })
  }
  return out
}

/** All unique notes and files made across a chat's messages and live turn. */
export function chatMadeItems(
  messages: { role: string; toolSteps?: StepLike[]; files?: ReplyFile[] }[],
  liveSteps: StepLike[] = [],
): { notes: MadeNote[]; files: ReplyFile[] } {
  const notes: MadeNote[] = []
  const files: ReplyFile[] = []
  const addSteps = (steps: StepLike[] | undefined) => {
    if (!steps) return
    for (const n of madeNotes(steps)) {
      if (!notes.some((x) => x.path === n.path)) notes.push(n)
    }
  }
  for (const m of messages) {
    if (m.role !== 'assistant') continue
    addSteps(m.toolSteps)
    for (const f of m.files ?? []) {
      if (!files.some((x) => x.path === f.path)) files.push(f)
    }
  }
  addSteps(liveSteps)
  return { notes, files }
}
