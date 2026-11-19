// The formats the note viewer offers for a vault note, and the small helpers around them.
// Names match the `format` query parameter of GET /api/note/export.

import { t } from '~/strings'
import { HqHttpError } from './hqAuth'
import { toastNotice, useToasts, type Toast } from './toast'
import { fetchNoteExport } from './vaultApi'

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

const SHEET_FENCE = /^\s{0,3}(```|~~~)\s*sheet(\s|$)/m

/** Whether the Markdown has a table, ignoring anything inside a fenced code block. A suite `sheet` block counts: it exports as a workbook. */
export function noteHasTables(markdown: string): boolean {
  if (SHEET_FENCE.test(markdown)) return true
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

/** The catalogue keys for one format: its name, what it is good for, and what to call the file while making it. */
export function exportKeys(format: NoteExportFormat) {
  return {
    name: `note_export_${format}`,
    hint: `note_export_${format}_hint`,
    kind: `note_export_kind_${format}`,
  } as const
}

export type ExportGate = 'ok' | 'ask' | 'window' | 'away'

/** What Export does now: HQ's window cannot save files, HQ away cannot make one, a shared computer asks first. */
export function exportGate(state: { inHub: boolean; away: boolean; shared: boolean }): ExportGate {
  if (state.inHub) return 'window'
  if (state.away) return 'away'
  return state.shared ? 'ask' : 'ok'
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

type ToastPatch = Omit<Toast, 'id' | 'at' | 'late'>

/** Replaces an in-flight toast if it is still visible, or pushes a new one if it already dismissed. */
export function replaceOrPushToast(id: number, next: ToastPatch): number {
  const store = useToasts.getState()
  if (store.toasts.some((x) => x.id === id)) {
    store.replace(id, next)
    return id
  }
  return store.push(next)
}

export interface RunNoteExportOptions {
  filePath: string
  format: NoteExportFormat
  isBusy: () => boolean
  setBusy: (format: NoteExportFormat | null) => void
  onFallbackPrintPdf: (toastId: number) => Promise<void>
  fetchExport?: typeof fetchNoteExport
  isTouch?: () => boolean
  canShareFile?: (file: File) => boolean
  shareFile?: (file: File, name: string) => Promise<void>
  downloadBlob?: (blob: Blob, name: string) => void
}

function defaultDownloadBlob(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

/** Opens the print fallback window for a note PDF, toasting if the browser blocked pop-ups. */
export function openPrintWindow(
  htmlDoc: string,
  toastId: number,
  openWindow: typeof window.open = (...args) => window.open(...args),
): boolean {
  const win = openWindow('', '_blank', 'width=900,height=700')
  if (!win) {
    replaceOrPushToast(toastId, { text: t('note_export_print_blocked'), kind: 'alert' })
    return false
  }
  win.document.write(htmlDoc)
  win.document.close()
  useToasts.getState().dismiss(toastId)
  return true
}

/** Runs a note export: keeps busy state outside the menu, shows a Making toast immediately, and replaces it on completion or failure. */
export async function runNoteExport(opts: RunNoteExportOptions): Promise<void> {
  if (opts.isBusy()) return
  const {
    filePath,
    format,
    setBusy,
    onFallbackPrintPdf,
    fetchExport = fetchNoteExport,
    isTouch = () => typeof window !== 'undefined' && Boolean(window.matchMedia?.('(pointer: coarse)').matches),
    canShareFile = (file) => typeof navigator !== 'undefined' && Boolean(navigator.canShare?.({ files: [file] })),
    shareFile = (file, name) => navigator.share({ files: [file], title: name }),
    downloadBlob = defaultDownloadBlob,
  } = opts

  setBusy(format)
  const kind = t(exportKeys(format).kind)
  const toastId = toastNotice(t('note_export_making', { kind }))
  try {
    const { blob, extension, kept } = await fetchExport(filePath, format)
    const name = exportFileName(filePath, extension)
    const file = new File([blob], name, { type: blob.type || 'application/octet-stream' })
    if (isTouch() && canShareFile(file)) {
      try {
        await shareFile(file, name)
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') {
          useToasts.getState().dismiss(toastId)
          return
        }
        throw err
      }
      replaceOrPushToast(toastId, { text: t('note_export_shared_done', { name }), kind: 'status' })
      return
    }
    downloadBlob(blob, name)
    const done = t('note_export_done', { name })
    replaceOrPushToast(toastId, { text: kept ? `${done} ${kept}` : done, kind: 'status' })
  } catch (err) {
    if (format === 'pdf' && err instanceof HqHttpError && err.status === 503) {
      await onFallbackPrintPdf(toastId)
    } else {
      replaceOrPushToast(toastId, {
        text: t('note_export_failed', { kind }),
        kind: 'alert',
        action: { label: t('note_export_try_again'), run: () => void runNoteExport(opts) },
      })
    }
  } finally {
    setBusy(null)
  }
}
