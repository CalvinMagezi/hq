import { t, tn } from '~/strings'
import { useCallback, useEffect, useState } from 'react'
import { ArrowLeft, Loader2, PanelLeftOpen, PanelRight } from 'lucide-react'
import { attachCommand, canSend, globalSessionsApi, isBlocked, type HarnessSession } from '~/lib/sessionsApi'
import { autoAnswers, type AutoAnswer } from '~/lib/autoAnswers'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { computerName, screenPollMs, sessionTitle } from '~/lib/workbench'
import { helperState } from './SessionRow'
import { TechnicalDetails } from './TechnicalDetails'
import { PaneHead } from '../ui/Panes'
import { SessionSendBox, useSend } from './SessionSendBox'
import { HelperPill, SessionTaskLink } from './SessionRow'
import { BlockedCallout } from './BlockedCallout'
import { ActionError, ArchiveButton, ResumeButton, useAction } from './SessionActions'

const DRIVE_ANSWERER = 'a Drive turn'
const clock = (when: string | null) => (when ? parseSqliteUtc(when).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '')

type Refresh = () => void | Promise<void>

interface Props {
  session: HarnessSession
  onBack: () => void
  onChanged: Refresh
  /** Opens the details pane below 1400 px. */
  onToggleSide?: () => void
  /** Brings back the list the owner collapsed. */
  onExpandList?: () => void
}

/** One helper: what it needs from you first, what it is doing, then the way to tell it more. Its screen stays closed until asked for. */
export function SessionDetail({ session: s, onBack, onChanged, onToggleSide, onExpandList }: Props) {
  const [sentCount, setSentCount] = useState(0)
  const [screenText, setScreenText] = useState('')
  const action = useAction(onChanged)
  const afterSend = useCallback(async () => {
    setSentCount((n) => n + 1)
    await onChanged()
  }, [onChanged])

  return (
    <div className="flex flex-col h-full min-h-0 overflow-y-auto overscroll-contain">
      <PaneHead title={sessionTitle(s)} back={
          <button type="button" onClick={onBack} className="a-ib sm a-back" aria-label={t('helpers_back')}>
            <ArrowLeft className="a-ic" aria-hidden="true" />
          </button>
        }>
        <HelperPill session={s} />
        {onExpandList && (
          <button type="button" onClick={onExpandList} className="a-ib sm" aria-label={t('helpers_expand')}>
            <PanelLeftOpen className="a-ic" aria-hidden="true" />
          </button>
        )}
        {onToggleSide && (
          <button type="button" onClick={onToggleSide} className="a-ib sm a-side-toggle" aria-label={t('helpers_details_aria')}>
            <PanelRight className="a-ic" aria-hidden="true" />
          </button>
        )}
      </PaneHead>
      <div className="a-wrap flex flex-col gap-2">
        <span className="a-body-sm a-muted">{computerName(s.host)}</span>
        <SessionTaskLink session={s} />
        <ActionError error={action.error} />
      </div>
      <Body session={s} screenText={screenText} onSent={afterSend} onChanged={onChanged} />
      <TechnicalDetails session={s} refreshKey={sentCount} pollMs={screenPollMs(s)} onText={setScreenText} onSent={afterSend} />
      <Answers sessionId={s.id} />
      <Advanced session={s} />
    </div>
  )
}

interface BodyProps {
  session: HarnessSession
  screenText: string
  onSent: Refresh
  onChanged: Refresh
}

function Body({ session: s, screenText, onSent, onChanged }: BodyProps) {
  if (s.status !== 'running') return <ResumePanel session={s} onChanged={onChanged} />
  if (!canSend(s)) {
    const lost = helperState(s) === 'lost'
    return (
      <p role="status" className="px-3 py-3 text-xs" style={{ color: 'var(--warn)' }}>
        {lost ? t('helpers_lost_note') : t('helpers_offline_note', { computer: computerName(s.host) })}
      </p>
    )
  }
  if (isBlocked(s)) return <BlockedCallout session={s} screenText={screenText} onSent={onSent} />
  return (
    <>
      <RightNow session={s} onSent={onSent} />
      <SessionSendBox session={s} onSent={onSent} />
    </>
  )
}

/** One line on what it is doing, and two quick replies in the phone's words. */
function RightNow({ session: s, onSent }: { session: HarnessSession; onSent: Refresh }) {
  const { send, busy, error } = useSend(s.id, onSent)
  const helper = sessionTitle(s)
  const ready = helperState(s) === 'ready'
  return (
    <section aria-label={t('helpers_right_now')} className="a-card">
      <p className="a-label">{t('helpers_right_now')}</p>
      <p className="a-body">{t(ready ? 'helpers_right_now_ready' : 'helpers_right_now_working', { helper })}</p>
      <div className="a-btns">
        <button type="button" disabled={busy} onClick={() => void send({ text: t('helpers_quick_keep_going_text') })} className="a-btn secondary">
          {t('helpers_quick_keep_going')}
        </button>
        <button type="button" disabled={busy} onClick={() => void send({ text: t('helpers_quick_explain_text') })} className="a-btn secondary">
          {t('helpers_quick_explain')}
        </button>
      </div>
      {error && <ErrorLine text={error} />}
    </section>
  )
}

const ErrorLine = ({ text }: { text: string }) => (
  <p role="alert" className="a-body-sm a-warn-t">
    {text}
  </p>
)

/** A stopped helper: when it stopped, what resuming does, and an optional next instruction. */
function ResumePanel({ session: s, onChanged }: { session: HarnessSession; onChanged: Refresh }) {
  const resume = useAction(onChanged)
  const archive = useAction(onChanged)
  const [next, setNext] = useState('')
  const helper = sessionTitle(s)
  const when = s.last_seen_at ? relTime(parseSqliteUtc(s.last_seen_at)) : ''
  return (
    <section aria-label={t('helpers_resume_act')} className="a-card">
      <div className="a-card-head">
        <b>{t('helpers_resume_title', { helper, when })}</b>
      </div>
      <p className="a-body-sm a-muted">{t('helpers_resume_why')}</p>
      <div className="a-field">
        <label htmlFor="resume-next" className="a-field-label">
          {t('helpers_resume_next')}
        </label>
        <textarea id="resume-next" rows={2} value={next} onChange={(e) => setNext(e.target.value)} className="a-input" />
      </div>
      <div className="a-btns">
        <ResumeButton session={s} action={resume} prompt={next} />
        {!s.archived && <ArchiveButton session={s} action={archive} />}
      </div>
      {resume.error && <ErrorLine text={`${t('helpers_resume_failed', { helper })} ${resume.error}`} />}
      <ActionError error={archive.error} />
    </section>
  )
}

/** What HQ answered for this helper, with the time of each. */
function Answers({ sessionId }: { sessionId: string }) {
  const [answers, setAnswers] = useState<AutoAnswer[] | null>(null)
  useEffect(() => {
    let live = true
    globalSessionsApi
      .answers(sessionId)
      .then((list) => live && setAnswers(autoAnswers(list)))
      .catch(() => live && setAnswers([]))
    return () => {
      live = false
    }
  }, [sessionId])
  if (answers === null) return <Loader2 className="a-ic sm animate-spin" aria-hidden="true" />
  return (
    <section aria-label={t('helpers_answers_title')} className="a-body-sm a-muted">
      <b>{tn('chat_say_yes_log', answers.length)}</b>
      {answers.length === 0 ? (
        <p>{t('helpers_answers_none')}</p>
      ) : (
        <ul>
          {answers.map((a, i) => (
            <li key={`${a.at}-${i}`}>{answerLine(a)}</li>
          ))}
        </ul>
      )}
    </section>
  )
}

function answerLine(a: AutoAnswer): string {
  const time = clock(a.at)
  if (a.by !== DRIVE_ANSWERER) return t('chat_say_yes_by', { time, by: a.by, kind: a.kind })
  return t(a.declined ? 'chat_say_yes_declined' : 'chat_say_yes_allowed', { time, kind: a.kind })
}

/** The command for checking this computer from a terminal. Most people never need it. */
function Advanced({ session: s }: { session: HarnessSession }) {
  return (
    <details>
      <summary className="a-btn quiet compact">{t('helpers_new_advanced')}</summary>
      <div>
        <pre className="a-quote">{attachCommand(s)}</pre>
      </div>
    </details>
  )
}
