import { t, tn } from '~/strings'
import { useCallback, useEffect, useState } from 'react'
import { ArrowLeft, Loader2 } from 'lucide-react'
import { attachCommand, canSend, globalSessionsApi, isBlocked, type HarnessSession } from '~/lib/sessionsApi'
import { autoAnswers, type AutoAnswer } from '~/lib/autoAnswers'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { computerName, folderName, screenPollMs, sessionTitle } from '~/lib/workbench'
import { helperState } from './SessionRow'
import { TechnicalDetails } from './TechnicalDetails'
import { SessionSendBox, useSend } from './SessionSendBox'
import { HelperPill, SessionTaskLink } from './SessionRow'
import { BlockedCallout } from './BlockedCallout'
import { StopAllButton } from './EmergencyStop'
import { ActionError, ArchiveButton, FollowInChat, RenameTitle, ResumeButton, StopButton, useAction } from './SessionActions'

const DRIVE_ANSWERER = 'a Drive turn'
const clock = (when: string | null) => (when ? parseSqliteUtc(when).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '')

type Refresh = () => void | Promise<void>

interface Props {
  session: HarnessSession
  onBack: () => void
  onChanged: Refresh
}

/** One helper: what it needs from you first, what it is doing, then the way to tell it more. Its screen stays closed until asked for. */
export function SessionDetail({ session: s, onBack, onChanged }: Props) {
  const [sentCount, setSentCount] = useState(0)
  const [screenText, setScreenText] = useState('')
  const action = useAction(onChanged)
  const running = s.status === 'running'
  const folder = folderName(s.cwd)
  const afterSend = useCallback(async () => {
    setSentCount((n) => n + 1)
    await onChanged()
  }, [onChanged])

  return (
    <div className="flex flex-col h-full min-h-0 overflow-y-auto overscroll-contain">
      <header className="px-3 py-2 border-b border-white/10 space-y-1.5 shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <button
            type="button"
            onClick={onBack}
            className="min-[1024px]:hidden flex items-center gap-1 min-h-11 -ml-2 px-2 rounded text-xs text-neutral-300 hover:text-white"
          >
            <ArrowLeft className="w-4 h-4" aria-hidden="true" />
            {t('helpers_back')}
          </button>
          <span className="min-[1024px]:hidden ml-auto">
            <StopAllButton compact onStopped={() => void onChanged()} />
          </span>
        </div>
        <RenameTitle session={s} action={action} />
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-400">
          <HelperPill session={s} />
          <span>{computerName(s.host)}</span>
          {folder && <span className="truncate">{t('helpers_folder_line', { folder })}</span>}
        </div>
        <SessionTaskLink session={s} />
        <div className="flex flex-wrap items-start gap-1.5 min-[1400px]:hidden">
          {running && <StopButton session={s} action={action} />}
          <FollowInChat session={s} action={action} />
        </div>
        <ActionError error={action.error} />
      </header>
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
    <section aria-label={t('helpers_right_now')} className="px-3 py-3 space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400">{t('helpers_right_now')}</h3>
      <p className="text-sm text-neutral-100">{t(ready ? 'helpers_right_now_ready' : 'helpers_right_now_working', { helper })}</p>
      <div className="flex flex-wrap gap-2">
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
  <p role="alert" className="text-xs" style={{ color: 'var(--warn)' }}>
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
    <section aria-label={t('helpers_resume_act')} className="px-3 py-3 space-y-2">
      <h3 className="text-sm font-semibold text-neutral-100">{t('helpers_resume_title', { helper, when })}</h3>
      <p className="text-xs text-neutral-300">{t('helpers_resume_why')}</p>
      <label htmlFor="resume-next" className="block text-xs text-neutral-400">
        {t('helpers_resume_next')}
      </label>
      <textarea id="resume-next" rows={2} value={next} onChange={(e) => setNext(e.target.value)} className="w-full px-3 py-2 rounded-xl text-sm hq-field" />
      <div className="flex flex-wrap gap-2">
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
  if (answers === null) return <Loader2 className="w-4 h-4 animate-spin m-3 text-neutral-500" aria-hidden="true" />
  return (
    <section aria-label={t('helpers_answers_title')} className="px-3 py-3 text-xs text-neutral-400">
      <h3 className="font-semibold text-neutral-300">{tn('chat_say_yes_log', answers.length)}</h3>
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
      <summary className="flex items-center min-h-11 px-3 cursor-pointer text-[11px] text-neutral-500 hover:text-neutral-300 select-none">{t('helpers_new_advanced')}</summary>
      <div className="space-y-1 px-3 pb-1">
        <code className="block px-1.5 py-1 rounded bg-black/40 text-[11px] text-neutral-300 break-all select-all">{attachCommand(s)}</code>
      </div>
    </details>
  )
}
