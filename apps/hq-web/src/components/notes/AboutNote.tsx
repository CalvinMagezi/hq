import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { t } from '~/strings'
import { relTime } from '~/lib/time'
import {
  currentVersion,
  fetchNoteVersions,
  offersLooksRight,
  reviewNote,
  reviewTag,
  whoWrote,
} from '~/lib/noteReview'
import { NoteHistory, whoText } from './NoteHistory'
import { ReviewTag } from './ReviewTag'

const LINES = {
  accepted: 'note_review_accepted_line',
  disputed: 'note_review_disputed_line',
  reviewed: 'note_review_reviewed_line',
  superseded: 'note_review_superseded_line',
  unknown: 'note_review_unknown_line',
  unreviewed: null,
} as const

/** The details panel: who wrote this version, whether anyone checked it, and the history. */
export function AboutNote({ path }: { path: string }) {
  const queryClient = useQueryClient()
  const key = ['note-versions', path]
  const versions = useQuery({ queryKey: key, queryFn: () => fetchNoteVersions(path), retry: false })
  const [history, setHistory] = useState(false)
  const [asking, setAsking] = useState(false)
  const [reason, setReason] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const answer = async (verdict: 'accepted' | 'disputed') => {
    if (verdict === 'disputed' && !reason.trim()) {
      setProblem(t('note_review_reason_needed'))
      return
    }
    setBusy(true)
    setProblem(null)
    try {
      await reviewNote(path, verdict, reason)
      setAsking(false)
      setReason('')
      await queryClient.invalidateQueries({ queryKey: key })
    } catch {
      setProblem(t('note_review_failed'))
    } finally {
      setBusy(false)
    }
  }

  let body
  if (versions.isPending) {
    body = <p className="a-muted a-body-sm" role="status">{t('note_about_loading')}</p>
  } else if (versions.isError) {
    body = (
      <div role="alert" className="a-body-sm" style={{ color: 'var(--warn)' }}>
        <p>{t('note_about_failed')}</p>
        <button type="button" className="a-btn secondary" onClick={() => void versions.refetch()}>{t('note_about_try_again')}</button>
      </div>
    )
  } else {
    const data = versions.data
    const now = currentVersion(data)
    const kind = reviewTag(now)
    const who = whoWrote(now)
    const line = LINES[kind]
    body = (
      <>
        <dl className="a-body-sm" style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '4px 16px', margin: 0 }}>
          <dt className="a-muted">{t('note_about_who')}</dt>
          <dd style={{ margin: 0 }}>{whoText(who)}</dd>
          <dt className="a-muted">{t('note_about_when')}</dt>
          <dd style={{ margin: 0 }}>{now ? relTime(now.written_at) : t('note_review_unknown')}</dd>
          <dt className="a-muted">{t('note_about_versions')}</dt>
          <dd style={{ margin: 0 }}>
            {data.versions.length}
            {data.versions.length > 0 && (
              <button type="button" className="a-link" style={{ marginLeft: 8 }} aria-expanded={history} onClick={() => setHistory(!history)}>
                {history ? t('note_about_hide_history') : t('note_about_show_history')}
              </button>
            )}
          </dd>
        </dl>
        <ReviewTag kind={kind} />
        {line && (
          <p className="a-body-sm a-muted">
            {kind === 'accepted' ? t(line, { when: relTime(now?.reviewed_at ?? now?.written_at) }) : t(line)}
          </p>
        )}
        {kind === 'disputed' && now?.review_reason && <q className="a-body-sm">{now.review_reason}</q>}
        {who.kind === 'you' && <p className="a-body-sm a-muted">{t('note_review_typed_line')}</p>}
        {history && <NoteHistory versions={data} />}
        {offersLooksRight(now) && (
          <>
            {asking && (
              <div className="a-field">
                <label className="a-field-label" htmlFor="note-review-reason">{t('note_review_reason_label')}</label>
                <textarea id="note-review-reason" className="a-input" rows={3} value={reason} onChange={(e) => setReason(e.target.value)} />
              </div>
            )}
            <div className="a-btns" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
              {asking ? (
                <>
                  <button type="button" className="a-btn secondary" disabled={busy} onClick={() => { setAsking(false); setProblem(null) }}>{t('note_review_reason_cancel')}</button>
                  <button type="button" className="a-btn" disabled={busy} onClick={() => void answer('disputed')}>{t('note_review_reason_send')}</button>
                </>
              ) : (
                <>
                  <button type="button" className="a-btn" disabled={busy} onClick={() => void answer('accepted')}>{t('note_review_looks_right')}</button>
                  <button type="button" className="a-btn secondary" disabled={busy} onClick={() => setAsking(true)}>{t('note_review_wrong')}</button>
                </>
              )}
            </div>
            <p className="a-body-sm a-muted">{t('note_about_either_way')}</p>
          </>
        )}
        {problem && <p role="alert" className="a-body-sm" style={{ color: 'var(--warn)' }}>{problem}</p>}
      </>
    )
  }

  return (
    <section aria-label={t('note_about_title')} className="a-card">
      <h2 className="a-h3">{t('note_about_title')}</h2>
      {body}
    </section>
  )
}
