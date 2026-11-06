import { useState } from 'react'
import { Loader2 } from 'lucide-react'
import { globalSessionsApi, type AnswerPayload, type HarnessSession } from '~/lib/sessionsApi'
import { answerErrorLine } from '~/lib/inboxCard'
import { toastNotice } from '~/lib/toast'
import { canAllowOnce } from '~/lib/helperView'
import { readDialog, sessionTitle } from '~/lib/workbench'
import { t } from '~/strings'
import { ErrorText } from './ErrorText'

type Done = () => unknown

/**
 * Answers a helper's dialog. The helper is read again first: when it now asks something else than
 * the question on screen, nothing is sent and the person is told to look again.
 */
export function useDialogAnswer(session: HarnessSession, onDone: Done) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const helper = sessionTitle(session)
  const answer = async (seq: number, payload: Omit<AnswerPayload, 'seq'>) => {
    if (busy) return false
    setBusy(true)
    setError(null)
    try {
      const fresh = await globalSessionsApi.get(session.id)
      if (fresh.pending_seq !== seq) {
        setError(t('helpers_look_again'))
        return false
      }
      const report = await globalSessionsApi.answer(session.id, { ...payload, seq })
      if (!report.registered) {
        setError(t('helpers_not_taken', { helper }))
        return false
      }
      toastNotice(t(payload.decline ? 'helpers_declined_toast' : 'helpers_allowed_toast', { helper }))
      return true
    } catch (err) {
      setError(answerErrorLine(err))
      return false
    } finally {
      // Stay busy until the refresh lands, so a second tap cannot answer the next dialog by mistake.
      await onDone()
      setBusy(false)
    }
  }
  return { answer, busy, error }
}

/** Decline and Allow once on a list card. Shown only for a plain yes or no question. */
export function InlineAnswer({ session, onDone, off = null }: { session: HarnessSession; onDone: Done; /** Why answering is off, said on the card. */ off?: string | null }) {
  const { answer, busy, error } = useDialogAnswer(session, onDone)
  const read = readDialog(session)
  if (!read || !canAllowOnce(read.dialog)) return null
  const picked = read.dialog.options.find((o) => o.selected)
  return (
    <>
      <div className="a-card-actions">
        <button type="button" disabled={busy || off !== null} onClick={() => void answer(read.seq, { decline: true })} className="a-btn secondary">
          {t('notice_action_decline')}
        </button>
        <button type="button" disabled={busy || off !== null} onClick={() => picked && void answer(read.seq, { option: picked.id })} className="a-btn">
          {busy && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
          {t('notice_action_allow_once')}
        </button>
      </div>
      {off && <p className="a-cap a-muted">{off}</p>}
      {error && <ErrorText>{error}</ErrorText>}
    </>
  )
}
