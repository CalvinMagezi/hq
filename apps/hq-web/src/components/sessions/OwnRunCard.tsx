import { useState } from 'react'
import { controlJob, jobLine, ownTitle, ownWords, type EdgeRun, type JobControl } from '~/lib/edgeWorker'
import { t } from '~/strings'

const OWN_RULE = { borderLeft: '3px solid var(--brand-indigo)' }

/** One of HQ's own coding jobs: where it stands, what it has spent, and what the owner can do. */
export function OwnRunCard({ run, onOpenChat, onChanged }: { run: EdgeRun; onOpenChat: () => void; onChanged: () => void }) {
  const [note, setNote] = useState('')
  const [failed, setFailed] = useState(false)
  const line = run.job ? jobLine(run.job) : null
  const send = async (action: JobControl) => {
    try {
      await controlJob(action, run.task_id ?? '', note.trim())
      setNote('')
      setFailed(false)
      onChanged()
    } catch {
      setFailed(true)
    }
  }
  const working = run.status === 'pending'
  const review = run.task?.status === 'ready_for_review'
  const blocked = run.task?.status === 'blocked'
  return (
    <div className="a-card w-full" style={OWN_RULE}>
      <b className="a-body-b truncate">{ownTitle(run)}</b>
      <span className="a-body-sm a-muted">{ownWords(run)}</span>
      {line && <span className="a-body-sm a-muted">{line}</span>}
      {run.notes?.map((text, n) => (
        <p key={n} className="a-body-sm">
          {text}
        </p>
      ))}
      {failed && <p className="a-body-sm a-warn-t">{t('github_failed')}</p>}
      <textarea aria-label={t('coder_note_hint')} placeholder={t('coder_note_hint')} className="a-input" rows={2} value={note} onChange={(e) => setNote(e.target.value)} />
      <div className="a-dlg-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        {run.pr_url && (
          <a className="a-btn compact" href={run.pr_url} target="_blank" rel="noopener noreferrer">
            {t('coder_open_pr')}
          </a>
        )}
        <button type="button" className="a-btn secondary compact" disabled={!run.thread_id} onClick={onOpenChat}>
          {t('coder_open_chat')}
        </button>
        {working && (
          <>
            <button type="button" className="a-btn secondary compact" disabled={!note.trim()} onClick={() => void send('steer')}>
              {t('coder_send_note')}
            </button>
            <button type="button" className="a-btn secondary compact" onClick={() => void send('stop')}>
              {t('coder_stop')}
            </button>
          </>
        )}
        {review && (
          <button type="button" className="a-btn secondary compact" disabled={!note.trim()} onClick={() => void send('feedback')}>
            {t('coder_send_back')}
          </button>
        )}
        {blocked && (
          <button type="button" className="a-btn secondary compact" onClick={() => void send('retry')}>
            {t('coder_retry')}
          </button>
        )}
      </div>
    </div>
  )
}
