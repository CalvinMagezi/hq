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

function text(args: Record<string, unknown> | undefined, key: string): string | null {
  const v = args?.[key]
  return typeof v === 'string' && v.trim() ? v.trim() : null
}

function baseName(path: string): string {
  const last = path.split('/').filter(Boolean).pop() ?? path
  return last.replace(/\.md$/, '')
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
    const at = seen.get(ref)
    if (at !== undefined) {
      out[at] = { ...out[at], reading: out[at].reading || reading }
      continue
    }
    seen.set(ref, out.length)
    out.push({ kind, title, ref, reading })
  }
  return out
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
