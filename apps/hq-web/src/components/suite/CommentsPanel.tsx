import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { t, tn } from '~/strings'
import { HqHttpError } from '~/lib/hqAuth'
import { useSharedConnection } from '~/lib/connection'
import { commentAdd, commentResolve, commentsIn, fetchBlocks, fetchComments, type SuiteBlock, type SuiteComment, type SuiteComments } from '~/lib/suiteApi'
import { discardCommentOp, mergeQueued, onCommentOpsChanged, queueCommentOp, refusedComments, replayLive, suiteOutbox, today, waitingComments, type CommentOp, type RefusedOp, type ShownComment, type WaitingOp } from '~/lib/commentOutbox'
import { readCommentCopy, saveCommentCopy } from '~/lib/commentCache'
import { currentHqId } from '~/lib/savedCopy'
import { isSharedComputer } from '~/lib/sharedComputer'
import { blockChoice, commentable, countComments, pickAnchor, splitComments, type AnchorHint } from '~/lib/commentView'
import { toastAlert, toastNotice } from '~/lib/toast'
import { Sheet } from '../ui/Sheet'
import './suite.css'

/** What the Comments button needs from a viewer: where the note lives, its text, and whether it may be changed. */
export interface CommentsProps {
  path: string
  /** The note as shown, for the count of open comments. */
  text: string
  /** The note may be changed: comments can be added and resolved. A view-only note only lists them. */
  canEdit: boolean
  /** A change is waiting to be sent; calls that go straight to HQ are held back until it is. */
  waiting?: boolean
  /** HQ wrote the note: load it again so the next save quotes the new hash. */
  onChanged: () => void
  /** Where the person is (the block, the card, the sheet), read when the sheet opens. */
  hint?: AnchorHint | (() => AnchorHint | undefined)
}

/** One call that changes the comments and answers with the new list. */
type Write = { (): Promise<{ comments?: unknown; revision?: unknown }> }

const BY_KEY = 'hq.suite.commentBy'
const readBy = () => {
  try {
    return localStorage.getItem(BY_KEY) ?? ''
  } catch {
    return ''
  }
}
const saveBy = (by: string) => {
  try {
    localStorage.setItem(BY_KEY, by)
  } catch {
    // The label is a convenience; without storage it is typed again.
  }
}

/** `2026-11-20` as the reader's own short date; anything else as written. */
function shortDate(at: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(at)
  if (!m) return at
  try {
    return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
  } catch {
    return at
  }
}

/** The comments of one note that wait to be sent, and those HQ refused, kept up to date as they change. */
function useCommentOps(path: string) {
  const [ops, setOps] = useState<{ waiting: WaitingOp[]; refused: RefusedOp[] }>({ waiting: [], refused: [] })
  const reload = useCallback(async () => {
    const id = currentHqId()
    if (!id) return setOps({ waiting: [], refused: [] })
    setOps({ waiting: await waitingComments(suiteOutbox, id, path), refused: await refusedComments(suiteOutbox, id, path) })
  }, [path])
  useEffect(() => {
    void reload()
    return onCommentOpsChanged(() => void reload())
  }, [reload])
  return ops
}

/** Whether a failed call means HQ could not be reached, rather than HQ saying no. */
const unreachable = (e: unknown) => !(e instanceof HqHttpError)

/** The Comments button with the count of open comments, and the sheet it opens. */
export function CommentsButton(props: CommentsProps) {
  const [shown, setShown] = useState<{ hint?: AnchorHint } | null>(null)
  const { open } = useMemo(() => countComments(props.text), [props.text])
  const connection = useSharedConnection()
  const { refused } = useCommentOps(props.path)
  const kept = refused.length > 0
  // While HQ can be reached, keep a copy of the comments and blocks so the sheet works later with no connection.
  useEffect(() => {
    if (!props.canEdit || connection !== 'connected') return
    void fetchComments(props.path).then((r) => saveCommentCopy(props.path, { comments: r.comments }), () => undefined)
    void fetchBlocks(props.path).then((r) => saveCommentCopy(props.path, { blocks: r.blocks }), () => undefined)
  }, [props.path, props.canEdit, connection])
  return (
    <>
      <button
        type="button"
        className="a-btn quiet compact su-cmt-btn"
        aria-haspopup="dialog"
        aria-label={kept ? t('suite_comments_kept_label') : open > 0 ? tn('suite_comments_open_label', open) : t('suite_comments')}
        onClick={() => setShown({ hint: typeof props.hint === 'function' ? props.hint() : props.hint })}
      >
        {t('suite_comments')}
        {open > 0 && (
          <span className="su-badge" aria-hidden="true">
            {open}
          </span>
        )}
        {kept && (
          <span className="su-badge su-badge-warn" aria-hidden="true">
            !
          </span>
        )}
      </button>
      {shown && <CommentsSheet {...props} hint={shown.hint} onClose={() => setShown(null)} />}
    </>
  )
}

function CommentsSheet({ path, canEdit, waiting, onChanged, hint, onClose }: Omit<CommentsProps, 'text' | 'hint'> & { hint?: AnchorHint; onClose: () => void }) {
  const connection = useSharedConnection()
  const held = !!waiting || connection !== 'connected'
  // Where HQ cannot be reached, or a note edit is still waiting, a comment waits on this device too. Not on a shared computer.
  const canQueue = canEdit && !isSharedComputer() && currentHqId() !== null
  const frozen = held && !canQueue
  const ops = useCommentOps(path)
  const [view, setView] = useState<SuiteComments | null>(null)
  const [blocks, setBlocks] = useState<SuiteBlock[] | null>(null)
  /** Where the list and the blocks came from: HQ now, the copy saved on this device, or nowhere. */
  const [source, setSource] = useState<'fresh' | 'saved' | 'none'>('fresh')
  const [blocksSource, setBlocksSource] = useState<'fresh' | 'saved' | 'none'>('fresh')
  const [message, setMessage] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [on, setOn] = useState('')
  const [draft, setDraft] = useState('')
  const [by, setBy] = useState(readBy)
  const online = useRef(connection === 'connected')
  online.current = connection === 'connected'

  const pickOn = useCallback(
    (list: SuiteBlock[], first: boolean) =>
      setOn((current) => {
        const choices = commentable(list)
        if (!first && choices.some((b) => b.hash === current)) return current
        return (pickAnchor(list, hint) ?? choices[0])?.hash ?? ''
      }),
    [hint],
  )
  const loadBlocks = useCallback(
    async (first: boolean) => {
      try {
        const r = await fetchBlocks(path)
        setBlocks(r.blocks)
        setBlocksSource('fresh')
        pickOn(r.blocks, first)
        void saveCommentCopy(path, { blocks: r.blocks })
      } catch (e) {
        const saved = unreachable(e) ? (await readCommentCopy(path))?.blocks : undefined
        setBlocks(saved ?? [])
        setBlocksSource(saved ? 'saved' : 'none')
        if (saved) pickOn(saved, first)
      }
    },
    [path, pickOn],
  )
  const load = useCallback(async () => {
    try {
      const r = await fetchComments(path)
      setView(r)
      setSource('fresh')
      setMessage(null)
      void saveCommentCopy(path, { comments: r.comments })
    } catch (e) {
      const saved = unreachable(e) ? (await readCommentCopy(path))?.comments : undefined
      setView({ revision: '', comments: saved ?? [] })
      setSource(saved ? 'saved' : 'none')
      // With no connection the saved copy (or the waiting ones alone) is the answer, not an error.
      setMessage(unreachable(e) ? null : t('suite_comments_failed'))
    }
    if (canEdit) await loadBlocks(true)
  }, [path, canEdit, loadBlocks])
  useEffect(() => {
    void load()
    // A replay changed what HQ holds: read it again.
    return onCommentOpsChanged(() => {
      if (online.current) void load()
    })
    // Read once when the sheet opens; a change made here updates the list from HQ's own answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  /** Keeps one comment operation on this device, and sends what waits at once when HQ can be reached. */
  const queue = async (op: CommentOp) => {
    const id = currentHqId()
    if (!id) return false
    const r = await queueCommentOp(suiteOutbox, id, op)
    if (!r.ok) {
      toastAlert(r.message)
      return false
    }
    toastNotice(t('suite_comment_queued'))
    if (online.current && !waiting) {
      void replayLive().then((res) => {
        if (res.sent.length > 0) onChanged()
        if (res.refused > 0) toastAlert(t('suite_comment_refused_toast'))
      })
    }
    return true
  }

  /** One write to HQ. A block or comment that changed since the list was read is refused: say so and read again. */
  const run = async (go: Write, done: string) => {
    if (held || busy) return false
    setBusy(true)
    setMessage(null)
    try {
      const r = await go()
      const next = commentsIn(r)
      if (next) {
        setView(next)
        setSource('fresh')
      }
      toastNotice(done)
      onChanged()
      void loadBlocks(false)
      return true
    } catch (e) {
      if (e instanceof HqHttpError && (e.status === 409 || e.status === 422)) setMessage(`${t('suite_comment_stale')}${e.message ? ` (${e.message})` : ''}`)
      else toastAlert(t('suite_comment_failed'))
      void load()
      return false
    } finally {
      setBusy(false)
    }
  }

  const shown: ShownComment[] = useMemo(() => mergeQueued(view?.comments ?? [], ops.waiting, blocks ?? []), [view, ops.waiting, blocks])
  const { open, resolved } = splitComments(shown)
  const choices = commentable(blocks ?? [])
  const text = draft.trim()
  // Anything already waiting goes first, so a comment is never sent ahead of an earlier one.
  const queuing = canQueue && (held || ops.waiting.length > 0)
  const submit = async () => {
    const label = by.trim()
    const ok = queuing ? await queue({ kind: 'comment-add', path, on, by: label, text, at: today() }) : await run(() => commentAdd(path, { on, text, ...(label ? { by: label } : {}) }), t('suite_comment_added'))
    if (ok) {
      saveBy(label)
      setDraft('')
    }
  }
  const resolve = (c: SuiteComment) => {
    if (queuing) void queue({ kind: 'comment-resolve', path, index: c.index, hash: c.hash, text: c.text })
    else void run(() => commentResolve(path, { index: c.index, hash: c.hash }), t('suite_comment_resolved'))
  }
  const discard = async (clientId: string) => {
    const id = currentHqId()
    if (id) await discardCommentOp(suiteOutbox, id, clientId)
  }

  return (
    <Sheet title={t('suite_comments')} onClose={onClose} placement="bottom" busy={busy}>
      {held && (
        <p role="status" className="a-body-sm a-warn-t">
          {canQueue ? t('suite_comments_offline') : t('suite_comments_offline_shared')}
        </p>
      )}
      {source !== 'fresh' && (
        <p role="status" className="a-body-sm a-muted">
          {source === 'saved' ? t('suite_comments_saved_copy') : t('suite_comments_no_copy')}
        </p>
      )}
      {message && (
        <p role="alert" className="a-body-sm a-warn-t">
          {message}
        </p>
      )}
      {ops.refused.map((r) => (
        <RefusedItem key={r.clientId} r={r} onDiscard={() => void discard(r.clientId)} />
      ))}
      {view === null ? (
        <p className="a-body-sm a-muted">{t('chat_pane_loading')}</p>
      ) : (
        <>
          {open.length === 0 && <p className="a-body-sm a-muted">{t('suite_comments_none')}</p>}
          <ul className="su-cmts">
            {open.map((c) => (
              <CommentItem
                key={c.waitingId ?? c.index}
                c={c}
                onResolve={canEdit && !c.waitingId && !c.resolving ? () => resolve(c) : undefined}
                onDiscard={c.waitingId ? () => void discard(c.waitingId as string) : undefined}
                disabled={frozen || busy}
              />
            ))}
          </ul>
          {resolved.length > 0 && (
            <details className="su-cmts-done">
              <summary className="a-body-sm-b">{tn('suite_comments_resolved', resolved.length)}</summary>
              <ul className="su-cmts">
                {resolved.map((c) => (
                  <CommentItem key={c.index} c={c} disabled />
                ))}
              </ul>
            </details>
          )}
        </>
      )}
      {canEdit && (
        <form
          className="su-form su-cmt-add"
          onSubmit={(e) => {
            e.preventDefault()
            if (text !== '' && on !== '' && !frozen) void submit()
          }}
        >
          <h3 className="a-body-b">{t('suite_comment_add_title')}</h3>
          {blocks !== null && choices.length === 0 ? (
            <p className="a-body-sm a-muted">{blocksSource === 'none' && held ? t('suite_comment_no_saved_blocks') : t('suite_comment_no_blocks')}</p>
          ) : (
            <label className="su-field">
              <span className="a-body-sm-b">{t('suite_comment_on')}</span>
              <select className="su-select" value={on} disabled={blocks === null || frozen} onChange={(e) => setOn(e.target.value)}>
                {choices.map((b) => {
                  const { kind, first } = blockChoice(b)
                  return (
                    <option key={b.hash} value={b.hash}>
                      {first ? `${kind}: ${first}` : kind}
                    </option>
                  )
                })}
              </select>
            </label>
          )}
          <label className="su-field">
            <span className="a-body-sm-b">{t('suite_comment_by')}</span>
            <input className="su-select" type="text" maxLength={60} value={by} autoComplete="off" onChange={(e) => setBy(e.target.value)} />
          </label>
          <label className="su-field">
            <span className="a-body-sm-b">{t('suite_comment_text')}</span>
            <textarea className="su-textarea" rows={3} maxLength={4000} value={draft} disabled={frozen} onChange={(e) => setDraft(e.target.value)} />
          </label>
          <button type="submit" className="a-btn" disabled={frozen || busy || text === '' || on === ''}>
            {t('suite_comment_add')}
          </button>
        </form>
      )}
    </Sheet>
  )
}

/** A comment HQ refused when it was sent after a wait: HQ's sentence, the person's words, and what to do with them. */
export function RefusedItem({ r, onDiscard }: { r: RefusedOp; onDiscard: () => void }) {
  const text = r.op.text ?? ''
  return (
    <div role="alert" className="su-conflict su-cmt-refused">
      <p className="a-body-sm a-warn-t">{t(r.op.kind === 'comment-add' ? 'suite_comment_refused_add' : 'suite_comment_refused_resolve', { reason: r.reason })}</p>
      {text && <blockquote className="su-cmt-anchor a-body-sm">{text}</blockquote>}
      <div className="su-step">
        {text && (
          <button
            type="button"
            className="a-btn quiet compact"
            onClick={() => {
              void navigator.clipboard?.writeText(text).then(
                () => toastNotice(t('suite_copied')),
                () => undefined,
              )
            }}
          >
            {t('suite_comment_copy_text')}
          </button>
        )}
        <button type="button" className="a-btn quiet compact" onClick={onDiscard}>
          {t('suite_comment_discard')}
        </button>
      </div>
    </div>
  )
}

export function CommentItem({ c, onResolve, onDiscard, disabled }: { c: ShownComment; onResolve?: () => void; onDiscard?: () => void; disabled: boolean }) {
  const who = c.by?.trim() || t('suite_comment_unsigned')
  return (
    <li className={`su-cmt${c.state === 'resolved' ? ' is-resolved' : ''}${c.waitingId || c.resolving ? ' is-waiting' : ''}`}>
      <p className="a-cap a-muted su-cmt-meta">{c.at ? `${who} · ${shortDate(c.at)}` : who}</p>
      {(c.anchor_preview || c.orphaned) && (
        <blockquote className="su-cmt-anchor a-body-sm">
          {c.orphaned && <b className="su-cmt-orphan">{t('suite_comment_orphaned')} </b>}
          {c.anchor_preview}
        </blockquote>
      )}
      <p className="a-body su-cmt-text">{c.text}</p>
      {(c.waitingId || c.resolving) && <p className="a-cap a-warn-t su-cmt-wait">{c.waitingId ? t('suite_comment_waiting') : t('suite_comment_resolve_waiting')}</p>}
      {onResolve && c.state !== 'resolved' && (
        <button type="button" className="a-btn quiet compact" disabled={disabled} onClick={onResolve}>
          {t('suite_comment_resolve')}
        </button>
      )}
      {onDiscard && (
        <button type="button" className="a-btn quiet compact" onClick={onDiscard}>
          {t('suite_comment_discard')}
        </button>
      )}
    </li>
  )
}
