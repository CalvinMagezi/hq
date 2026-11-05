import { del, get, set } from 'idb-keyval'
import { t } from '~/strings'
import { keyFor as keyedName } from './savedCopy'

export type OutboxKind = 'message' | 'note' | 'task'

export interface OutboxEntry {
  client_id: string
  kind: OutboxKind
  hq_id: string
  thread_id?: string
  payload: unknown
  created_at: number
  attempts: number
  last_reason?: string
  /** A chat message past its 24 h: kept in the thread, never sent by the drain. */
  waited?: boolean
  /** When the owner pressed Send now on a waited message; its 24 h count starts again from here. */
  resent_at?: number
}

export interface OutboxStore {
  load(hqId: string): Promise<OutboxEntry[]>
  save(hqId: string, entries: OutboxEntry[]): Promise<void>
}

export type SendResult = 'sent' | 'retry' | 'gone'
export type AddResult = { ok: true } | { ok: false; reason: 'outbox_full'; message: string }
export type OutboxLineState = 'starting' | 'unreachable' | 'offline' | 'next'

export const OUTBOX_LIMIT = 100
export const MESSAGE_MAX_AGE_MS = 24 * 60 * 60 * 1000

export const OUTBOX_NAME = 'hq-outbox'
const OUTBOX_LOCK = 'hq-outbox'
const keyFor = (hqId: string) => keyedName(OUTBOX_NAME, hqId)

export const browserStore: OutboxStore = {
  async load(hqId) {
    if (typeof indexedDB === 'undefined') return []
    try {
      return ((await get<OutboxEntry[]>(keyFor(hqId))) ?? []).slice()
    } catch {
      return []
    }
  },
  async save(hqId, entries) {
    if (typeof indexedDB === 'undefined') return
    try {
      if (entries.length === 0) await del(keyFor(hqId))
      else await set(keyFor(hqId), entries)
    } catch {
      // A browser that refuses storage simply keeps nothing.
    }
  },
}

/** Made when the owner presses Send or Save, never on a retry. */
export function newClientId(): string {
  const c = globalThis.crypto
  if (typeof c?.randomUUID === 'function') return c.randomUUID()
  const bytes = new Uint8Array(16)
  c.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

export function lineFor(state: OutboxLineState, inHub: boolean): string {
  return t(`outbox_line_${state}${inHub ? '_window' : ''}` as const)
}

/** Rule 14: on a shared computer nothing waits. */
export function outboxEnabled(shared: boolean): boolean {
  return !shared
}

export function sharedSendLine(): string {
  return t('outbox_shared_send')
}

export function createOutbox(store: OutboxStore, now: () => number = Date.now) {
  // Every change reads, edits and writes the whole list, so changes run one at a time: a message
  // added while the drain is sending must never be written over by the drain's older copy.
  let chain: Promise<unknown> = Promise.resolve()
  function locked<T>(fn: () => Promise<T>): Promise<T> {
    // Tabs share the store: a Web Lock keeps one tab's change from writing over another's.
    const across = (): Promise<T> =>
      typeof navigator !== 'undefined' && navigator.locks ? (navigator.locks.request(OUTBOX_LOCK, fn) as Promise<T>) : fn()
    const run = chain.then(across, across)
    chain = run.catch(() => undefined)
    return run
  }

  async function list(hqId: string): Promise<OutboxEntry[]> {
    const all = await store.load(hqId)
    return all.filter((e) => e.hq_id === hqId).sort((a, b) => a.created_at - b.created_at)
  }

  function change(hqId: string, edit: (entries: OutboxEntry[]) => OutboxEntry[]): Promise<OutboxEntry[]> {
    return locked(async () => {
      const next = edit(await list(hqId))
      await store.save(hqId, next)
      return next
    })
  }

  function add(entry: OutboxEntry): Promise<AddResult> {
    return locked(async () => {
      const entries = await list(entry.hq_id)
      if (entries.length >= OUTBOX_LIMIT) {
        return { ok: false, reason: 'outbox_full', message: t('fail_add_outbox_full') } as const
      }
      await store.save(entry.hq_id, [...entries, entry])
      return { ok: true } as const
    })
  }

  async function remove(hqId: string, clientId: string): Promise<void> {
    await change(hqId, (entries) => entries.filter((e) => e.client_id !== clientId))
  }

  async function clear(hqId: string): Promise<void> {
    await change(hqId, () => [])
  }

  /** Clears the waited mark so the next drain sends it ("Send now"); its place in the order stays. */
  async function sendNow(hqId: string, clientId: string): Promise<void> {
    const at = now()
    await change(hqId, (entries) =>
      entries.map((e) => (e.client_id === clientId ? { ...e, waited: false, resent_at: at } : e)),
    )
  }

  const expired = (e: OutboxEntry, at: number) =>
    e.kind === 'message' && !e.waited && at - (e.resent_at ?? e.created_at) > MESSAGE_MAX_AGE_MS

  /** Sends what waits, one at a time, oldest first; stops at the first that must wait longer. */
  async function drain(
    hqId: string,
    send: (entry: OutboxEntry) => Promise<SendResult>,
    onProgress?: (n: number, m: number) => void,
    only?: OutboxKind,
  ): Promise<void> {
    const at = now()
    const entries = await change(hqId, (list) => list.map((e) => (expired(e, at) ? { ...e, waited: true } : e)))
    const queue = entries.filter((e) => !e.waited && (only === undefined || e.kind === only))
    for (const [i, entry] of queue.entries()) {
      onProgress?.(i + 1, queue.length)
      const result = await send(entry)
      if (result === 'retry') {
        await change(hqId, (list) =>
          list.map((e) => (e.client_id === entry.client_id ? { ...e, attempts: e.attempts + 1 } : e)),
        )
        return
      }
      await remove(hqId, entry.client_id)
    }
  }

  return { list, add, remove, clear, sendNow, drain }
}

export type Outbox = ReturnType<typeof createOutbox>
