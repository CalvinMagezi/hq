// Structure edits for sheets, sent to HQ's one edit route (`POST /api/suite/edit`, see suiteApi.ts).
//
// Every call quotes the sheet block's hash, read just before the call, so HQ refuses an edit made
// from a page that is out of date (a 409/422 with a sentence). Rows and columns inserted or deleted
// move the formulas, tiles and plots that read what lies beyond them; HQ does that and says how many.
import { fetchBlocks, sheetAddColumn, sheetAddRows, sheetDeleteColumn, sheetDeleteRows, sheetSetColumnType } from './suiteApi'

export type OpResult = { ok: true } | { ok: false; reason: 'stale' | 'failed'; message: string }

/** Which note and which sheet block (counting only `sheet` blocks, from 0). `base` is unused: the hash is read fresh. */
export interface Target {
  path: string
  base?: string
  index: number
}

export interface SuiteOps {
  /** Inserts a blank row at `at` (a data row number from 0), or deletes the row there. */
  rows(t: Target, action: 'insert' | 'delete', at: number): Promise<OpResult>
  /** Adds a column named `name` before column `at` (from 0), or on the right when `at` is left out. */
  addColumn(t: Target, name: string, at?: number): Promise<OpResult>
  /** Deletes column `col` (from 0). */
  deleteColumn(t: Target, col: number): Promise<OpResult>
  columnType(t: Target, col: number, type: string): Promise<OpResult>
}

async function sheetHash(t: Target): Promise<string> {
  const { blocks } = await fetchBlocks(t.path)
  const sheet = blocks.filter((b) => b.kind === 'sheet')[t.index]
  if (!sheet) throw new Error('that note has no such sheet')
  return sheet.hash
}

async function guarded(t: Target, call: (hash: string) => Promise<unknown>): Promise<OpResult> {
  try {
    await call(await sheetHash(t))
    return { ok: true }
  } catch (e) {
    const message = e instanceof Error ? e.message : ''
    // HQ answers a stale hash or a refused change with a plain sentence; an outage has none.
    return { ok: false, reason: /changed|refus|must|cannot|not a|no such/i.test(message) ? 'stale' : 'failed', message }
  }
}

export const suiteOps: SuiteOps = {
  rows: (t, action, at) =>
    guarded(t, (hash) =>
      action === 'insert'
        ? sheetAddRows(t.path, { sheet: t.index, hash, rows: [['']], at: at + 1 })
        : sheetDeleteRows(t.path, { sheet: t.index, hash, row: at + 1, count: 1 }),
    ),
  addColumn: (t, name, at) => guarded(t, (hash) => sheetAddColumn(t.path, { sheet: t.index, hash, name, ...(at === undefined ? {} : { at }) })),
  deleteColumn: (t, col) => guarded(t, (hash) => sheetDeleteColumn(t.path, { sheet: t.index, hash, col })),
  columnType: (t, col, type) => guarded(t, (hash) => sheetSetColumnType(t.path, { sheet: t.index, hash, col, type })),
}

/** Kept for the screens that ask whether HQ can do a thing: every call here is always offered. */
export function useSuiteOps(): { ops: SuiteOps } {
  return { ops: suiteOps }
}
