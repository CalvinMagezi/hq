import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { globalSessionsApi, type DialogKind, type HarnessSession, type PendingDialog } from '~/lib/sessionsApi'
import { ANSWER_HOLD_MS, answerHeld, classifyBlocked, readDialog, sessionTitle, tailLines, tappableOptions } from '~/lib/workbench'
import { canAllowOnce, computerInLine } from '~/lib/helperView'
import { useDialogAnswer } from './HelperAnswer'
import { MoreKeys, SendError, useSend } from './SessionSendBox'

const CALLOUT_LINES = 12
const BIG_BUTTON_CLASS =
  'flex-1 flex items-center justify-center gap-2 h-12 rounded-xl border border-white/10 bg-white/5 text-sm font-semibold hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-white/5'
const OPTION_CLASS =
  'w-full flex flex-col items-start gap-0.5 min-h-12 px-4 py-2 rounded-xl border border-white/10 bg-white/5 text-left text-sm font-semibold hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-white/5'

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
    <section
      aria-label={t('helpers_waiting_for_you')}
      className="hq-composer border-t px-3 py-3 space-y-2"
      style={{ borderColor: 'var(--warn)' }}
    >
      {!read && (
        <h3 className="text-sm font-semibold" style={{ color: 'var(--warn)' }}>
          {t('helpers_waiting_for_you')}
        </h3>
      )}
      {read ? (
        <DialogAnswer session={session} dialog={read.dialog} seq={read.seq} onSent={onSent} />
      ) : (
        <>
          <p className="text-xs text-neutral-400">{t('helpers_unreadable')}</p>
          {kind !== 'approval' && <p className="text-xs text-neutral-400">{t('helpers_answer_in_terminal')}</p>}
          {tail && (
            <pre className="max-h-40 overflow-auto overscroll-contain rounded-lg bg-black/40 px-3 py-2 text-xs leading-snug text-neutral-200 whitespace-pre-wrap break-words">
              {tail}
            </pre>
          )}
          {kind === 'approval' && (
            <div className="flex items-center gap-2">
              <button type="button" disabled={held} onClick={() => answer('esc')} className={BIG_BUTTON_CLASS}>
                {t('notice_action_decline')}
              </button>
              <button type="button" disabled={held} onClick={() => answer('enter')} className={BIG_BUTTON_CLASS}>
                {busy && <Loader2 className="w-4 h-4 animate-spin" />}
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
    <div className="space-y-3">
      <h3 className="text-base font-semibold text-neutral-50">{t(HEADS[dialog.kind] as 'helpers_wants_run', { helper })}</h3>
      {dialog.question && <p className="text-sm text-neutral-100 break-words">{dialog.question}</p>}
      {dialog.command && (
        <div>
          <p className="text-xs text-neutral-400">{t(dialog.kind === 'edit' ? 'helpers_quote_head_edit' : 'helpers_quote_head', { helper })}</p>
          <pre className="a-quote mt-1 max-h-48 overflow-auto overscroll-contain whitespace-pre-wrap break-words">{dialog.command}</pre>
        </div>
      )}
      {dialog.truncated && <p className="text-xs text-neutral-500">{t('helpers_shortened')}</p>}
      <p className="text-xs text-neutral-300">
        <span className="font-semibold">{t('approval_in_words')}: </span>
        {t(IN_WORDS[dialog.kind] as 'helpers_in_words_run', { computer: computerInLine(session.host) })}
      </p>
      {allow && (
        <>
          <p className="text-xs text-neutral-400">{t('helpers_if_allow', { helper })}</p>
          <p className="text-xs text-neutral-400">{t('helpers_if_decline', { helper })}</p>
          <div className="flex flex-wrap gap-2">
            <button type="button" disabled={busy} onClick={() => void answer(seq, { decline: true })} className="a-btn secondary flex-1 min-w-[8rem]">
              {t('notice_action_decline')}
            </button>
            <button type="button" disabled={busy} onClick={() => picked && void answer(seq, { option: picked.id })} className="a-btn flex-1 min-w-[8rem]">
              {busy && <Loader2 className="w-4 h-4 animate-spin" aria-hidden="true" />}
              {t('notice_action_allow_once')}
            </button>
          </div>
          <p className="text-xs text-neutral-500">{t('helpers_allow_note')}</p>
        </>
      )}
      {!allow && (
        <button type="button" disabled={busy} onClick={() => void answer(seq, { decline: true })} className={OPTION_CLASS}>
          {t('notice_action_decline')}
        </button>
      )}
      {others.length > 0 && (
        <div className="space-y-2">
          <p className="text-xs font-semibold text-neutral-300">{t('approval_other_answers')}</p>
          {others.map((o) => (
            <button key={o.id} type="button" disabled={busy} onClick={() => void answer(seq, { option: o.id })} className={OPTION_CLASS} style={o.remember ? { color: 'var(--warn)' } : undefined}>
              <span>{o.label}</span>
              {o.remember && <span className="text-xs font-normal text-neutral-400">{t('helpers_other_answers_hint')}</span>}
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
          className="space-y-2"
          onSubmit={(e) => {
            e.preventDefault()
            if (message.trim()) void answer(seq, { decline: true, message: message.trim() })
          }}
        >
          <label className="block text-xs text-neutral-300" htmlFor="tell-instead">
            {t('helpers_tell_label', { helper })}
          </label>
          <input id="tell-instead" value={message} onChange={(e) => setMessage(e.target.value)} className="w-full h-11 px-3 rounded-lg text-sm hq-field" />
          <button type="submit" disabled={busy || !message.trim()} className="a-btn">
            {t('helpers_tell_send')}
          </button>
        </form>
      )}
      {error && (
        <p role="alert" className="text-xs" style={{ color: 'var(--warn)' }}>
          {error}
        </p>
      )}
    </div>
  )
}
