import { hqJson } from './hqAuth'

/** Where one office file stands for the HQ Suite (`GET /api/suite/file`); hq writes every sentence. */
export interface SuiteFile {
  path: string
  state: 'ready' | 'prepared' | 'shrunk'
  job: string | null
  note: string | null
  saved_bytes: number | null
  headline: string
  details: string[]
  actions: { shrink: boolean; confirm: boolean; abandon: boolean; undo: boolean }
}

export const fetchSuiteFile = (path: string) => hqJson<SuiteFile>(`/api/suite/file?path=${encodeURIComponent(path)}`)
export const suiteShrink = (path: string) => hqJson<SuiteFile>('/api/suite/shrink', 'POST', { path })
export const suiteConfirm = (job: string) => hqJson<SuiteFile>('/api/suite/confirm', 'POST', { job })
export const suiteAbandon = (job: string) => hqJson<SuiteFile>('/api/suite/abandon', 'POST', { job })
export const suiteUndo = (job: string) => hqJson<SuiteFile>('/api/suite/undo', 'POST', { job })

/**
 * The office file a note was made from, read from its front matter: the note names a `source` and the
 * hash of what it was made from, and does not say its original is gone. Null for any other note.
 */
export function suiteSourceOf(text: string): string | null {
  const lines = text.split('\n')
  if (lines[0]?.trim() !== '---') return null
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === '---')
  if (end < 0) return null
  const keys = new Map<string, string>()
  for (const line of lines.slice(1, end)) {
    const at = line.indexOf(':')
    if (at <= 0 || /^\s/.test(line)) continue
    keys.set(line.slice(0, at).trim(), line.slice(at + 1).trim().replace(/^["']|["']$/g, ''))
  }
  const source = keys.get('source')
  if (!source || !keys.get('source_sha256')) return null
  return keys.get('suite_original_state') === 'gone' ? null : source
}

// ---- Sheet values worked out by hq (added for the web grid; kept apart from the file calls above) ----

/** One window of `GET /api/suite/sheet`: hq's own answer for cells, header row included at row 0. */
export interface ServerSheetView {
  name: string
  row0: number
  col0: number
  row_count: number
  col_count: number
  rows: { text: string; raw: string; formula: boolean; number: boolean; error: boolean }[][]
}

export const fetchSheetWindow = (path: string, index: number, row: number, col: number) =>
  hqJson<ServerSheetView>(`/api/suite/sheet?path=${encodeURIComponent(path)}&index=${index}&row=${row}&col=${col}`)

/** One block of a note as `GET /api/suite/blocks` lists it. */
export interface SuiteBlock {
  index: number
  hash: string
  kind: string
  src: string
}
export interface SuiteBlocks {
  revision: string
  blocks: SuiteBlock[]
}
export const fetchBlocks = (path: string) => hqJson<SuiteBlocks>(`/api/suite/blocks?path=${encodeURIComponent(path)}`)
export const editBlock = (path: string, index: number, hash: string, markdown: string) =>
  hqJson<SuiteBlocks>('/api/suite/blocks/edit', 'POST', { path, index, hash, markdown })
export const insertBlock = (path: string, markdown: string, after?: { index: number; hash: string }) =>
  hqJson<SuiteBlocks>('/api/suite/blocks/insert', 'POST', { path, markdown, ...(after ?? {}) })
export const deleteBlock = (path: string, index: number, hash: string) =>
  hqJson<SuiteBlocks>('/api/suite/blocks/delete', 'POST', { path, index, hash })

// ---------------------------------------------------------------- edits (`POST /api/suite/edit`)

/** What an edit answers: a summary hq words, the note's new revision and the action's own fields. */
export interface SuiteEditResult {
  /** The edit was written. */
  changed?: boolean
  /** A dry run: what the edit would do. Nothing was written. */
  preview?: boolean
  summary: string
  revision: string
  touched?: number
  destructive?: boolean
  [field: string]: unknown
}

/** A deck's card as `GET /api/suite/cards` lists it. Quote `hash` back with any card edit. */
export interface SuiteCard {
  index: number
  title: string
  hash: string
  blocks: number
  layout: string
  transition: string | null
  background: string | null
  color: string | null
  notes: string[]
}

const snake = (key: string) => key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`)

/**
 * Sends one edit. `args` use camelCase here and snake_case on the wire (`afterIndex` becomes
 * `after_index`); a refusal (a stale hash, a formula that reads moved rows) rejects with hq's sentence
 * and nothing is written. `dryRun` asks what the edit would do without doing it.
 */
export function suiteEdit(path: string, action: string, args: Record<string, unknown> = {}, dryRun = false) {
  const body: Record<string, unknown> = { path, action }
  for (const [key, value] of Object.entries(args)) if (value !== undefined) body[snake(key)] = value
  if (dryRun) body.dry_run = true
  return hqJson<SuiteEditResult>('/api/suite/edit', 'POST', body)
}

type Cells = (string | number | boolean | null)[][]
type Edit<A> = (path: string, args: A, dryRun?: boolean) => Promise<SuiteEditResult>
const edit =
  <A extends object>(name: string): Edit<A> =>
  (path, args, dryRun) =>
    suiteEdit(path, name, args as Record<string, unknown>, dryRun)

/** Sheet: `sheet` is the sheet block's number from 0; `hash` is that block's hash from the blocks list. */
export const sheetSetCells = edit<{ sheet?: number; row: number; col: number; values: Cells; expected: Cells }>('set_cells')
export const sheetAddRows = edit<{ sheet?: number; hash: string; rows: Cells; at?: number }>('add_rows')
export const sheetDeleteRows = edit<{ sheet?: number; hash: string; row: number; count?: number; expected?: Cells }>('delete_rows')
export const sheetAddColumn = edit<{ sheet?: number; hash: string; name: string; type?: string; default?: string }>('add_column')
export const sheetSetColumnType = edit<{ sheet?: number; hash: string; col: number; type: string }>('set_column_type')
export const sheetAdd = edit<{ name: string; columns: string[]; types?: string[]; rows?: Cells; afterIndex?: number; hash?: string }>('add_sheet')

/** Slides: a card is named by its number and the hash `fetchSuiteCards` lists. */
export const fetchSuiteCards = (path: string) =>
  hqJson<{ revision: string; title: string; cards: SuiteCard[] }>(`/api/suite/cards?path=${encodeURIComponent(path)}`)
export const cardAdd = edit<{ title: string; markdown?: string; afterCard?: number; hash?: string }>('add_card')
export const cardReplace = edit<{ card: number; hash: string; title?: string; markdown: string }>('replace_card')
export const cardMove = edit<{ card: number; hash: string; to: number }>('move_card')
export const cardDelete = edit<{ card: number; hash: string }>('delete_card')
export const cardSetNotes = edit<{ card: number; hash: string; text: string }>('set_notes')
/** A key given as `null` is cleared; one left out is not touched. */
export const cardSetStyle = edit<{
  card: number
  hash: string
  transition?: string | null
  background?: string | null
  color?: string | null
  layout?: string | null
}>('set_card_style')

/** Dashboard: a tile or a chart from a range; hq draws it first and refuses one that would not draw. */
export const dashAddTile = edit<{ from: string; fn?: string; where?: string; label?: string; format?: string; afterIndex?: number; hash?: string }>('add_tile')
export const dashAddPlot = edit<{ type: string; from: string; x: string; y: string; where?: string; group?: string; title?: string; afterIndex?: number; hash?: string }>('add_plot')

// ---------------------------------------------------------------- review comments

/** One review comment as the `comments` action lists it. A comment is a `comment` block anchored to another block by its hash. */
export interface SuiteComment {
  /** Position of the comment block; quote it back with `hash` to resolve it. */
  index: number
  hash: string
  /** The hash of the block it is about. */
  on: string
  /** A label the writer typed, never an identity. */
  by: string | null
  /** An ISO date. */
  at: string | null
  state: 'open' | 'resolved' | string
  text: string
  /** The block it was about has been edited or removed since. */
  orphaned: boolean
  /** The first line of the block it is about. */
  anchor_preview: string | null
}
export interface SuiteComments {
  revision: string
  comments: SuiteComment[]
}

/** The note's comments, in the order they are written. A read: nothing is written and no hash is needed. */
export const fetchComments = (path: string) => suiteEdit(path, 'comments') as unknown as Promise<SuiteComments>

/** Adds a comment on the block whose hash is `on`. `by` is a label and `at` a date (YYYY-MM-DD); hq fills `at` when left out. */
export const commentAdd = edit<{ on: string; text: string; by?: string; at?: string }>('add_comment')
/** Marks one comment resolved; refused when the comment changed since it was listed. */
export const commentResolve = edit<{ index: number; hash: string }>('resolve_comment')

/** The comment list an add or a resolve answers with, or null when the answer carries none. */
export function commentsIn(result: { comments?: unknown; revision?: unknown }): SuiteComments | null {
  if (!Array.isArray(result.comments) || typeof result.revision !== 'string') return null
  return { revision: result.revision, comments: result.comments as SuiteComment[] }
}
