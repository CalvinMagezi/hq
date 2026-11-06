import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { globalSessionsApi, type DialogKind, type HarnessSession, type PendingDialog } from '~/lib/sessionsApi'
import { ANSWER_HOLD_MS, answerHeld, classifyBlocked, readDialog, sessionTitle, tailLines, tappableOptions } from '~/lib/workbench'
import { canAllowOnce, computerInLine } from '~/lib/helperView'
import { useDialogAnswer } from './HelperAnswer'
import { ErrorText } from './ErrorText'
import { MoreKeys, SendError, useSend } from './SessionSendBox'

const CALLOUT_LINES = 12
const OPTION_CLASS = 'a-btn secondary block'

const HEADS: Record<DialogKind, string> = {
  command: 'helpers_wants_run',
  edit: 'helpers_wants_edit',
  path: 'helpers_wants_path',
  question: 'helpers_wants_question',
  trust: 'helpers_wants_trust',
  other: 'helpers_wants_other',
}

const IN_WORDS: Record<DialogKind, string> = {
  command: 'helpers_in_words_run',
  edit: 'helpers_in_words_edit',
  path: 'helpers_in_words_path',
  question: 'helpers_in_words_other',
  trust: 'helpers_in_words_other',
  other: 'helpers_in_words_other',
}

interface Props {
  session: HarnessSession
  /** The latest terminal text, so the question is readable right here. */
  screenText: string
  onSent: () => void | Promise<void>
}

/** The common case: the agent asked and waits. A dialog HQ can read gets one button per option;
 *  otherwise Approve and Decline appear only for a plain yes or no question. */
export function BlockedCallout({ session, screenText, onSent }: Props) {
  const { send, busy, error } = useSend(session.id, onSent)
  const [once, setOnce] = useState('')
  // Technical details starts closed, so the question is read once here instead of from a stream.
  useEffect(() => {
    if (screenText) return
    let live = true
    globalSessionsApi
      .screen(session.id, 40)
      .then((r) => live && setOnce(r.lines.join('\n')))
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [session.id, screenText])
  const text = screenText || once
  const tail = tailLines(text, CALLOUT_LINES)
  const kind = classifyBlocked(text)
  const read = readDialog(session)
  const [answered, setAnswered] = useState<string | null>(null)
  const held = busy || answerHeld(answered, tail)

  useEffect(() => {
    if (answered === null) return
    const timer = setTimeout(() => setAnswered(null), ANSWER_HOLD_MS)
    return () => clearTimeout(timer)
  }, [answered])

  const answerWith: typeof send = (payload) => {
    setAnswered(tail)
    return send(payload)
  }
  const answer = (key: string) => void answerWith({ keys: [key] })

  return (
    <section aria-label={t('helpers_waiting_for_you')} className="a-card needs">
      {!read && <h3 className="a-body-b a-warn-t">{t('helpers_waiting_for_you')}</h3>}
      {read ? (
        <DialogAnswer session={session} dialog={read.dialog} seq={read.seq} onSent={onSent} />
      ) : (
        <>
          <p className="a-body-sm a-muted">{t('helpers_unreadable')}</p>
          {kind !== 'approval' && <p className="a-body-sm a-muted">{t('helpers_answer_in_terminal')}</p>}
          {tail && <pre className="a-quote max-h-40 overflow-auto overscroll-contain break-words">{tail}</pre>}
          {kind === 'approval' && (
            <div className="a-card-actions a-btns">
              <button type="button" disabled={held} onClick={() => answer('esc')} className="a-btn secondary">
                {t('notice_action_decline')}
              </button>
              <button type="button" disabled={held} onClick={() => answer('enter')} className="a-btn">
                {busy && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
                {t('notice_action_allow_once')}
              </button>
            </div>
          )}
        </>
      )}
      <MoreKeys send={answerWith} disabled={held} defaultOpen={!read && kind !== 'approval'} />
      <SendError error={error} />
    </section>
  )
}

/** The answer card: what it would run, in HQ's words, what each answer does, and Decline before Allow once. */
function DialogAnswer({ session, dialog, seq, onSent }: { session: HarnessSession; dialog: PendingDialog; seq: number; onSent: () => void | Promise<void> }) {
  const { answer, busy, error } = useDialogAnswer(session, onSent)
  const [telling, setTelling] = useState(false)
  const [message, setMessage] = useState('')
  const helper = sessionTitle(session)
  const allow = canAllowOnce(dialog)
  const picked = dialog.options.find((o) => o.selected)
  const others = tappableOptions(dialog).filter((o) => !(allow && o.id === picked?.id))
  const typed = dialog.options.some((o) => o.typed)
  return (
    <>
      <h3 className="a-body-b">{t(HEADS[dialog.kind] as 'helpers_wants_run', { helper })}</h3>
      {dialog.question && <p className="a-body-sm break-words">{dialog.question}</p>}
      {dialog.command && (
        <>
          <p className="a-label">{t(dialog.kind === 'edit' ? 'helpers_quote_head_edit' : 'helpers_quote_head', { helper })}</p>
          <pre className="a-quote max-h-48 overflow-auto overscroll-contain break-words">{dialog.command}</pre>
        </>
      )}
      {dialog.truncated && <p className="a-cap a-muted">{t('helpers_shortened')}</p>}
      <p className="a-body-sm a-muted">
        <b>{t('approval_in_words')}: </b>
        {t(IN_WORDS[dialog.kind] as 'helpers_in_words_run', { computer: computerInLine(session.host) })}
      </p>
      {allow && (
        <>
          <p className="a-body-sm">{t('helpers_if_allow', { helper })}</p>
          <p className="a-body-sm">{t('helpers_if_decline', { helper })}</p>
          <div className="a-card-actions a-btns">
            <button type="button" disabled={busy} onClick={() => void answer(seq, { decline: true })} className="a-btn secondary">
              {t('notice_action_decline')}
            </button>
            <button type="button" disabled={busy} onClick={() => picked && void answer(seq, { option: picked.id })} className="a-btn">
              {busy && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
              {t('notice_action_allow_once')}
            </button>
          </div>
          <p className="a-cap a-muted">{t('helpers_allow_note')}</p>
        </>
      )}
      {!allow && (
        <button type="button" disabled={busy} onClick={() => void answer(seq, { decline: true })} className={OPTION_CLASS}>
          {t('notice_action_decline')}
        </button>
      )}
      {others.length > 0 && (
        <div className="flex flex-col gap-2">
          <p className="a-label">{t('approval_other_answers')}</p>
          {others.map((o) => (
            <button key={o.id} type="button" disabled={busy} onClick={() => void answer(seq, { option: o.id })} className={OPTION_CLASS}>
              <span>{o.label}</span>
              {o.remember && <span className="a-cap a-warn-t">{t('helpers_other_answers_hint')}</span>}
            </button>
          ))}
        </div>
      )}
      {typed && !telling && (
        <button type="button" disabled={busy} onClick={() => setTelling(true)} className={OPTION_CLASS}>
          {t('helpers_tell_instead', { helper })}
        </button>
      )}
      {telling && (
        <form
          className="a-field"
          onSubmit={(e) => {
            e.preventDefault()
            if (message.trim()) void answer(seq, { decline: true, message: message.trim() })
          }}
        >
          <label className="a-field-label" htmlFor="tell-instead">
            {t('helpers_tell_label', { helper })}
          </label>
          <input id="tell-instead" value={message} onChange={(e) => setMessage(e.target.value)} className="a-input" />
          <button type="submit" disabled={busy || !message.trim()} className="a-btn">
            {t('helpers_tell_send')}
          </button>
        </form>
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </>
  )
}
