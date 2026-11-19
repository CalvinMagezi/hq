// The files a reply made, as the server saved them in the reply's meta. The server checked each
// path against the vault and read its name, type and size from the file, so nothing here is
// taken from the model's own words.

export type FileKind = 'sheet' | 'doc' | 'slides' | 'pdf' | 'image' | 'file'

export interface ReplyFile {
  path: string
  name: string
  mime: string
  kind: FileKind
  size: number
  /** The note this file was made from, when it was made by an export. */
  note?: string
  /** One plain line: what the export kept. */
  fidelity?: string
}

const KINDS: FileKind[] = ['sheet', 'doc', 'slides', 'pdf', 'image', 'file']

export function toReplyFiles(raw: unknown): ReplyFile[] | undefined {
  if (!Array.isArray(raw)) return undefined
  const files: ReplyFile[] = []
  for (const f of raw) {
    if (!f || typeof f !== 'object') continue
    const o = f as Record<string, unknown>
    if (typeof o.path !== 'string' || typeof o.name !== 'string') continue
    files.push({
      path: o.path,
      name: o.name,
      mime: typeof o.mime === 'string' ? o.mime : 'application/octet-stream',
      kind: KINDS.includes(o.kind as FileKind) ? (o.kind as FileKind) : 'file',
      size: typeof o.size === 'number' ? o.size : 0,
      note: typeof o.note === 'string' ? o.note : undefined,
      fidelity: typeof o.fidelity === 'string' ? o.fidelity : undefined,
    })
  }
  return files.length > 0 ? files : undefined
}

const KB = 1024
const MB = KB * KB

export function sizeLabel(bytes: number): string {
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`
  return `${bytes} B`
}
