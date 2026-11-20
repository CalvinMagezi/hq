import { useCallback, useEffect, useMemo, useState } from 'react'
import { t, tn } from '~/strings'
import { HqHttpError } from '~/lib/hqAuth'
import { useSharedConnection } from '~/lib/connection'
import { commentAdd, commentResolve, commentsIn, fetchBlocks, fetchComments, type SuiteBlock, type SuiteComment, type SuiteComments } from '~/lib/suiteApi'
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

/** The Comments button with the count of open comments, and the sheet it opens. */
export function CommentsButton(props: CommentsProps) {
  const [shown, setShown] = useState<{ hint?: AnchorHint } | null>(null)
  const { open } = useMemo(() => countComments(props.text), [props.text])
  return (
    <>
      <button
        type="button"
        className="a-btn quiet compact su-cmt-btn"
        aria-haspopup="dialog"
        aria-label={open > 0 ? tn('suite_comments_open_label', open) : t('suite_comments')}
        onClick={() => setShown({ hint: typeof props.hint === 'function' ? props.hint() : props.hint })}
      >
        {t('suite_comments')}
        {open > 0 && (
          <span className="su-badge" aria-hidden="true">
            {open}
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
  const [view, setView] = useState<SuiteComments | null>(null)
  const [blocks, setBlocks] = useState<SuiteBlock[] | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [on, setOn] = useState('')
  const [draft, setDraft] = useState('')
  const [by, setBy] = useState(readBy)

  const loadBlocks = useCallback(
    async (first: boolean) => {
      try {
        const r = await fetchBlocks(path)
        setBlocks(r.blocks)
        setOn((current) => {
          const list = commentable(r.blocks)
          if (!first && list.some((b) => b.hash === current)) return current
          return (pickAnchor(r.blocks, hint) ?? list[0])?.hash ?? ''
        })
      } catch {
        setBlocks([])
      }
    },
    [path, hint],
  )
  const load = useCallback(async () => {
    try {
      setView(await fetchComments(path))
      setMessage(null)
    } catch {
      setView({ revision: '', comments: [] })
      setMessage(t('suite_comments_failed'))
    }
    if (canEdit) await loadBlocks(true)
  }, [path, canEdit, loadBlocks])
  useEffect(() => {
    void load()
    // Read once when the sheet opens; a change made here updates the list from HQ's own answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  /** One write to HQ. A block or comment that changed since the list was read is refused: say so and read again. */
  const run = async (go: Write, done: string) => {
    if (held || busy) return false
    setBusy(true)
    setMessage(null)
    try {
      const r = await go()
      const next = commentsIn(r)
      if (next) setView(next)
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

  const { open, resolved } = splitComments(view?.comments ?? [])
  const choices = commentable(blocks ?? [])
  const text = draft.trim()
  const submit = async () => {
    const label = by.trim()
    const ok = await run(() => commentAdd(path, { on, text, ...(label ? { by: label } : {}) }), t('suite_comment_added'))
    if (ok) {
      saveBy(label)
      setDraft('')
    }
  }

  return (
    <Sheet title={t('suite_comments')} onClose={onClose} placement="bottom" busy={busy}>
      {held && (
        <p role="status" className="a-body-sm a-warn-t">
          {t('suite_comments_offline')}
        </p>
      )}
      {message && (
        <p role="alert" className="a-body-sm a-warn-t">
          {message}
        </p>
      )}
      {view === null ? (
        <p className="a-body-sm a-muted">{t('chat_pane_loading')}</p>
      ) : (
        <>
          {open.length === 0 && <p className="a-body-sm a-muted">{t('suite_comments_none')}</p>}
          <ul className="su-cmts">
            {open.map((c) => (
              <CommentItem key={c.index} c={c} onResolve={canEdit ? () => void run(() => commentResolve(path, { index: c.index, hash: c.hash }), t('suite_comment_resolved')) : undefined} disabled={held || busy} />
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
            if (text !== '' && on !== '') void submit()
          }}
        >
          <h3 className="a-body-b">{t('suite_comment_add_title')}</h3>
          {blocks !== null && choices.length === 0 ? (
            <p className="a-body-sm a-muted">{t('suite_comment_no_blocks')}</p>
          ) : (
            <label className="su-field">
              <span className="a-body-sm-b">{t('suite_comment_on')}</span>
              <select className="su-select" value={on} disabled={blocks === null || held} onChange={(e) => setOn(e.target.value)}>
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
            <textarea className="su-textarea" rows={3} maxLength={4000} value={draft} disabled={held} onChange={(e) => setDraft(e.target.value)} />
          </label>
          <button type="submit" className="a-btn" disabled={held || busy || text === '' || on === ''}>
            {t('suite_comment_add')}
          </button>
        </form>
      )}
    </Sheet>
  )
}

function CommentItem({ c, onResolve, disabled }: { c: SuiteComment; onResolve?: () => void; disabled: boolean }) {
  const who = c.by?.trim() || t('suite_comment_unsigned')
  return (
    <li className={`su-cmt${c.state === 'resolved' ? ' is-resolved' : ''}`}>
      <p className="a-cap a-muted su-cmt-meta">{c.at ? `${who} · ${shortDate(c.at)}` : who}</p>
      {(c.anchor_preview || c.orphaned) && (
        <blockquote className="su-cmt-anchor a-body-sm">
          {c.orphaned && <b className="su-cmt-orphan">{t('suite_comment_orphaned')} </b>}
          {c.anchor_preview}
        </blockquote>
      )}
      <p className="a-body su-cmt-text">{c.text}</p>
      {onResolve && c.state !== 'resolved' && (
        <button type="button" className="a-btn quiet compact" disabled={disabled} onClick={onResolve}>
          {t('suite_comment_resolve')}
        </button>
      )}
    </li>
  )
}
