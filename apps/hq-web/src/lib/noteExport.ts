// The formats the note viewer offers for a vault note, and the small helpers around them.
// Names match the `format` query parameter of GET /api/note/export.

export type NoteExportFormat = 'pdf' | 'docx' | 'html' | 'md' | 'png' | 'svg' | 'xlsx' | 'csv' | 'json'

export type NoteExportGroup = 'document' | 'image' | 'data'

export interface NoteExportOption {
  format: NoteExportFormat
  label: string
  /** The usual file extension. The server may answer with another (a zip for several tables). */
  extension: string
  group: NoteExportGroup
  /** Only offered when the note has a table to export. */
  needsTables?: boolean
}

export const NOTE_EXPORT_OPTIONS: readonly NoteExportOption[] = [
  { format: 'pdf', label: 'PDF document', extension: 'pdf', group: 'document' },
  { format: 'docx', label: 'Word document', extension: 'docx', group: 'document' },
  { format: 'html', label: 'Web page', extension: 'html', group: 'document' },
  { format: 'md', label: 'Markdown', extension: 'md', group: 'document' },
  { format: 'png', label: 'Image (whole note)', extension: 'png', group: 'image' },
  { format: 'svg', label: 'Vector image', extension: 'svg', group: 'image' },
  { format: 'xlsx', label: 'Excel workbook', extension: 'xlsx', group: 'data', needsTables: true },
  { format: 'csv', label: 'CSV', extension: 'csv', group: 'data', needsTables: true },
  { format: 'json', label: 'JSON', extension: 'json', group: 'data', needsTables: true },
]

const FENCE = /^\s{0,3}(```|~~~)/
// A GFM table's delimiter row: `| --- | :-: |`, or as short as `|:--|--:|`. The caller also requires
// a pipe on it, which keeps a plain `---` (a rule, or the end of frontmatter) from counting.
const DELIMITER_ROW = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/

/** Whether the Markdown has a table, ignoring anything inside a fenced code block. */
export function noteHasTables(markdown: string): boolean {
  let fence: string | null = null
  let previous = ''
  for (const line of markdown.split('\n')) {
    const marker = FENCE.exec(line)?.[1]
    if (marker) {
      if (fence === null) fence = marker
      else if (marker === fence) fence = null
      previous = ''
      continue
    }
    if (fence !== null) continue
    if (previous.includes('|') && line.includes('|') && DELIMITER_ROW.test(line)) return true
    previous = line
  }
  return false
}

/** The options worth showing for this note: table formats only when it has a table. */
export function exportOptionsFor(markdown: string): NoteExportOption[] {
  const tables = noteHasTables(markdown)
  return NOTE_EXPORT_OPTIONS.filter((o) => !o.needsTables || tables)
}

export function defaultExtension(format: NoteExportFormat): string {
  return NOTE_EXPORT_OPTIONS.find((o) => o.format === format)?.extension ?? format
}

/** `Notebooks/Inbox/Plan.md` + `docx` -> `Plan.docx`, with characters a file name cannot hold removed. */
export function exportFileName(notePath: string, extension: string): string {
  const base = (notePath.split('/').pop() ?? '').replace(/\.(md|markdown|txt)$/i, '')
  const safe = base.replace(/[\\/:*?"<>|\u0000-\u001f]/g, '-').trim()
  return `${safe || 'note'}.${extension}`
}

/**
 * The extension of the file the server named in Content-Disposition, or `fallback`.
 * A note with several tables exports as a .zip even when CSV was asked for, so the saved file
 * has to follow what the server sent, not what was requested.
 */
export function extensionFromDisposition(header: string | null, fallback: string): string {
  if (!header) return fallback
  let name: string | null = null
  const encoded = /filename\*\s*=\s*[^']*'[^']*'([^;]+)/i.exec(header)?.[1]
  if (encoded) {
    try {
      name = decodeURIComponent(encoded.trim())
    } catch {
      name = null
    }
  }
  name ??= /filename\s*=\s*"([^"]*)"/i.exec(header)?.[1] ?? /filename\s*=\s*([^;\s]+)/i.exec(header)?.[1] ?? null
  const ext = name?.split('.').pop()?.toLowerCase()
  return ext && /^[a-z0-9]{1,8}$/.test(ext) && name!.includes('.') ? ext : fallback
}
