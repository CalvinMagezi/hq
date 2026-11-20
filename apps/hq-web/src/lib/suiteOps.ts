// Structure edits for sheets and slides, called through one interface so the screens never name a route.
//
// ROUTES PENDING THE BACKEND BRANCH. The routes below are PLANNED and were not in this tree when this
// file was written (see docs/techdebt/suite-ops-routes-pending.md). Until they exist the server answers
// 404; the first probe notices that and `useSuiteOps` reports each route as unavailable, so the screens
// hide the buttons that need it. When the backend lands, check the bodies here against its handlers.
import { useEffect, useState } from 'react'
import { hqFetch } from './hqAuth'

export type OpName = 'rows' | 'columns' | 'columnType' | 'card' | 'move' | 'notes' | 'style'

/** PENDING: planned route for each operation. */
export const OP_ROUTES: Record<OpName, string> = {
  rows: '/api/suite/sheet/rows',
  columns: '/api/suite/sheet/columns',
  columnType: '/api/suite/sheet/column-type',
  card: '/api/suite/slides/card',
  move: '/api/suite/slides/move',
  notes: '/api/suite/slides/notes',
  style: '/api/suite/slides/style',
}

export type OpResult = { ok: true } | { ok: false; reason: 'unavailable' | 'stale' | 'failed'; message: string }

/** Every call names the note and the hash this page last saw, so a note that moved on is refused. */
export interface Target {
  path: string
  base?: string
  /** Which sheet block, or which card, depending on the call. */
  index: number
}

export interface SuiteOps {
  /** Inserts (`count` > 0) or deletes rows at `at`, a data row number from 0. */
  rows(t: Target, action: 'insert' | 'delete', at: number, count?: number): Promise<OpResult>
  columns(t: Target, action: 'insert' | 'delete', at: number): Promise<OpResult>
  columnType(t: Target, col: number, type: string): Promise<OpResult>
  card(t: Target, action: 'add' | 'duplicate' | 'delete'): Promise<OpResult>
  move(t: Target, to: number): Promise<OpResult>
  notes(t: Target, text: string): Promise<OpResult>
  style(t: Target, style: { layout?: string; transition?: string; background?: string; color?: string }): Promise<OpResult>
}

type Fetcher = (url: string, init?: RequestInit) => Promise<Response>

async function post(fetcher: Fetcher, name: OpName, body: unknown): Promise<OpResult> {
  let res: Response
  try {
    res = await fetcher(OP_ROUTES[name], { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
  } catch {
    return { ok: false, reason: 'failed', message: '' }
  }
  if (res.ok) return { ok: true }
  if (res.status === 404 || res.status === 405) {
    markAvailability(name, false)
    return { ok: false, reason: 'unavailable', message: '' }
  }
  const b = (await res.json().catch(() => ({}))) as { error?: string }
  return { ok: false, reason: res.status === 409 || res.status === 422 ? 'stale' : 'failed', message: b.error ?? '' }
}

/** The fetch-based implementation. `fetcher` is injectable for tests. */
export function createSuiteOps(fetcher: Fetcher = hqFetch): SuiteOps {
  const base = (t: Target) => ({ path: t.path, base: t.base, index: t.index })
  return {
    rows: (t, action, at, count = 1) => post(fetcher, 'rows', { ...base(t), action, at, count }),
    columns: (t, action, at) => post(fetcher, 'columns', { ...base(t), action, at }),
    columnType: (t, col, type) => post(fetcher, 'columnType', { ...base(t), col, type }),
    card: (t, action) => post(fetcher, 'card', { ...base(t), action }),
    move: (t, to) => post(fetcher, 'move', { ...base(t), to }),
    notes: (t, text) => post(fetcher, 'notes', { ...base(t), text }),
    style: (t, style) => post(fetcher, 'style', { ...base(t), ...style }),
  }
}

export const suiteOps: SuiteOps = createSuiteOps()

// ---- which routes exist ----

type Known = Partial<Record<OpName, boolean>>
let known: Known = {}
let probing: Promise<void> | null = null
const listeners = new Set<() => void>()

function markAvailability(name: OpName, yes: boolean) {
  if (known[name] === yes) return
  known = { ...known, [name]: yes }
  listeners.forEach((l) => l())
}

/**
 * Asks once whether each route exists. OPTIONS never changes anything: a missing route answers 404, an
 * existing one answers 405 or 2xx. A probe that cannot reach HQ at all leaves the answer unknown (shown).
 */
export function probeSuiteOps(fetcher: Fetcher = hqFetch): Promise<void> {
  if (probing) return probing
  probing = Promise.all(
    (Object.keys(OP_ROUTES) as OpName[]).map(async (name) => {
      try {
        const res = await fetcher(OP_ROUTES[name], { method: 'OPTIONS' })
        markAvailability(name, res.status !== 404)
      } catch {
        // Offline: stay unknown.
      }
    }),
  ).then(() => undefined)
  return probing
}

/** Test hook: forget what was learned. */
export function resetSuiteOpsProbe() {
  known = {}
  probing = null
}

/** Whether a route may be used: true until a probe or a call found it missing. */
export function opAvailable(name: OpName): boolean {
  return known[name] !== false
}

/** Re-renders when a route is found missing, and probes once. */
export function useSuiteOps(): { available: (name: OpName) => boolean; ops: SuiteOps } {
  const [, tick] = useState(0)
  useEffect(() => {
    const l = () => tick((n) => n + 1)
    listeners.add(l)
    void probeSuiteOps()
    return () => {
      listeners.delete(l)
    }
  }, [])
  return { available: opAvailable, ops: suiteOps }
}
