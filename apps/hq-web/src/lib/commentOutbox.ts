// Review comments made while HQ cannot be reached (or while a note edit is still waiting) are kept as small
// operations in the same outbox as waiting note edits and sent, oldest first, when HQ is back. A comment is
// added against the block hash the person saw, and resolved against the comment hash they saw; either can be
// stale by then. HQ refuses a stale one with a sentence, and the person's text is kept beside that sentence
// until they copy or discard it. Nothing is retried forever and nothing is dropped silently.
import { browserStore, createOutbox, newClientId, type Outbox, type OutboxEntry } from './outbox'
import { HqHttpError } from './hqAuth'
import { currentHqId } from './savedCopy'
import { commentAdd, commentResolve, fetchComments, type SuiteBlock, type SuiteComment, type SuiteComments } from './suiteApi'

/** The one outbox this page uses for suite edits and comments, so a change in one is never written over by the other. */
export const suiteOutbox: Outbox = createOutbox(browserStore)

export interface CommentAddOp {
  kind: 'comment-add'
  path: string
  /** Hash of the block the comment is about, as the blocks list gave it. */
  on: string
  by: string
  text: string
  /** A date, YYYY-MM-DD, fixed when the person pressed Add, so a replay names the same comment. */
  at: string
}
export interface CommentResolveOp {
  kind: 'comment-resolve'
  path: string
  index: number
  hash: string
  /** The comment's words, kept only so a refusal can show which comment it was. */
  text?: string
}
export type CommentOp = CommentAddOp | CommentResolveOp

const isOp = (e: OutboxEntry): boolean => {
  const k = (e.payload as CommentOp | undefined)?.kind
  return e.kind === 'comment' && (k === 'comment-add' || k === 'comment-resolve')
}
/** A refusal is an entry marked waited, so the drain never sends it; `last_reason` is HQ's sentence. */
const isRefused = (e: OutboxEntry) => isOp(e) && e.waited === true
const isWaiting = (e: OutboxEntry) => isOp(e) && !e.waited

const listeners = new Set<() => void>()
/** Called after any change to the waiting or refused comments, so an open panel can read them again. */
export function onCommentOpsChanged(fn: () => void): () => void {
  listeners.add(fn)
  return () => void listeners.delete(fn)
}
const changed = () => listeners.forEach((fn) => fn())

/** Today as YYYY-MM-DD in the person's own day. */
export function today(now: Date = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`
}

export type Queued = { ok: true } | { ok: false; message: string }

/** Keeps one comment operation for later. */
export async function queueCommentOp(outbox: Outbox, hqId: string, op: CommentOp, now: () => number = Date.now): Promise<Queued> {
  const added = await outbox.add({ client_id: newClientId(), kind: 'comment', hq_id: hqId, payload: op, created_at: now(), attempts: 0 })
  changed()
  return added.ok ? { ok: true } : { ok: false, message: added.message }
}

export interface WaitingOp {
  clientId: string
  op: CommentOp
}
export interface RefusedOp extends WaitingOp {
  /** HQ's sentence for why it was not applied. */
  reason: string
}

/** What waits to be sent for one note, oldest first. */
export async function waitingComments(outbox: Outbox, hqId: string, path: string): Promise<WaitingOp[]> {
  return (await outbox.list(hqId)).filter((e) => isWaiting(e) && (e.payload as CommentOp).path === path).map((e) => ({ clientId: e.client_id, op: e.payload as CommentOp }))
}

/** What HQ refused for one note, kept for the person to copy or discard. */
export async function refusedComments(outbox: Outbox, hqId: string, path: string): Promise<RefusedOp[]> {
  return (await outbox.list(hqId))
    .filter((e) => isRefused(e) && (e.payload as CommentOp).path === path)
    .map((e) => ({ clientId: e.client_id, op: e.payload as CommentOp, reason: e.last_reason ?? '' }))
}

/** The person keeps or drops a waiting or refused operation. */
export async function discardCommentOp(outbox: Outbox, hqId: string, clientId: string): Promise<void> {
  await outbox.remove(hqId, clientId)
  changed()
}

/** The calls a replay makes: the same functions the panel uses when HQ is reachable. */
export interface CommentApi {
  list: (path: string) => Promise<SuiteComments>
  add: (path: string, args: { on: string; text: string; by?: string; at?: string }) => Promise<unknown>
  resolve: (path: string, args: { index: number; hash: string }) => Promise<unknown>
}

export interface CommentReplay {
  /** Notes whose comments were written. */
  sent: string[]
  /** Operations HQ refused; each is kept with its sentence. */
  refused: number
}

/** The same comment, for an add that may already have been applied: same block, label, words and date. */
export const sameComment = (c: SuiteComment, op: CommentAddOp) => c.on === op.on && (c.by ?? '') === op.by && c.text.trim() === op.text.trim() && (c.at ?? '') === op.at

/** The calls to HQ itself. */
export const liveCommentApi: CommentApi = { list: fetchComments, add: commentAdd, resolve: commentResolve }

/** Sends what waits for this HQ with the live calls; with no known HQ nothing is sent. */
export async function replayLive(): Promise<CommentReplay> {
  const id = currentHqId()
  return id ? replayComments(suiteOutbox, id, liveCommentApi) : { sent: [], refused: 0 }
}

const inflight = new WeakMap<Outbox, Promise<CommentReplay>>()

/**
 * Sends what waits, oldest first. An add first reads the comments and is skipped when HQ already has it (a
 * send that got through but whose answer was lost). A refusal (a stale block or comment) keeps the operation
 * as a visible refusal; an unreachable HQ, or one that errors, leaves it for the next try.
 */
export function replayComments(outbox: Outbox, hqId: string, api: CommentApi): Promise<CommentReplay> {
  const running = inflight.get(outbox)
  if (running) return running
  const go = run(outbox, hqId, api).finally(() => {
    inflight.delete(outbox)
    changed()
  })
  inflight.set(outbox, go)
  return go
}

/** HQ said no to this one on its merits (the note moved on, the request is not valid): do not try again. */
const refusal = (e: unknown): e is HqHttpError => e instanceof HqHttpError && [400, 403, 404, 409, 410, 422].includes(e.status)

async function run(outbox: Outbox, hqId: string, api: CommentApi): Promise<CommentReplay> {
  const result: CommentReplay = { sent: [], refused: 0 }
  await outbox.drain(
    hqId,
    async (entry) => {
      if (!isWaiting(entry)) return 'gone'
      const op = entry.payload as CommentOp
      try {
        const have = (await api.list(op.path)).comments
        if (op.kind === 'comment-add') {
          if (!have.some((c) => sameComment(c, op))) await api.add(op.path, { on: op.on, text: op.text, ...(op.by ? { by: op.by } : {}), at: op.at })
        } else {
          // Already resolved (by an earlier send whose answer was lost, or by someone else): nothing to do.
          if (have.find((c) => c.index === op.index)?.state !== 'resolved') await api.resolve(op.path, { index: op.index, hash: op.hash })
        }
        result.sent.push(op.path)
        return 'sent'
      } catch (e) {
        // Unreachable, or HQ had a bad moment: keep it and try at the next reconnect.
        if (!refusal(e)) return 'retry'
        result.refused += 1
        await outbox.add({ ...entry, client_id: newClientId(), waited: true, last_reason: e.message || String(e.status) })
        return 'gone'
      }
    },
    undefined,
    'comment',
  )
  return result
}

/** A comment as the panel draws it: HQ's own, or one still waiting to be sent. */
export interface ShownComment extends SuiteComment {
  /** Set on one that waits to be added: its outbox id, so it can be taken back. */
  waitingId?: string
  /** Set on one whose resolve waits to be sent. */
  resolving?: boolean
}

/** The first line of a block, for the quote under a comment that is not on HQ yet. */
const firstLine = (src: string) => src.split('\n').find((l) => l.trim() !== '')?.trim() ?? null

/**
 * The comments HQ listed with the waiting operations laid over them: an add shows at once at the end of the
 * list, a resolve marks its comment. Negative indexes keep a waiting comment apart from HQ's own.
 */
export function mergeQueued(listed: SuiteComment[], waiting: WaitingOp[], blocks: SuiteBlock[] = []): ShownComment[] {
  const out: ShownComment[] = listed.map((c) => ({ ...c }))
  let n = 0
  for (const w of waiting) {
    const op = w.op
    if (op.kind === 'comment-resolve') {
      const hit = out.find((c) => c.index === op.index && c.hash === op.hash && !c.waitingId)
      if (hit) hit.resolving = true
    } else {
      // A replay that already reached HQ shows once: HQ's copy wins.
      if (listed.some((c) => sameComment(c, op))) continue
      n += 1
      out.push({
        index: -n,
        hash: '',
        on: op.on,
        by: op.by || null,
        at: op.at,
        state: 'open',
        text: op.text,
        orphaned: false,
        anchor_preview: firstLine(blocks.find((b) => b.hash === op.on)?.src ?? ''),
        waitingId: w.clientId,
      })
    }
  }
  return out
}
