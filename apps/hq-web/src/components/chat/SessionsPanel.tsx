import { useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Archive, EyeOff } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import './chat.css'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { agentName, computerName } from '~/lib/names'
import { t, tn } from '~/strings'
import { autoAnswers, type AutoAnswer } from '~/lib/autoAnswers'
import { Dialog } from '../ui/Dialog'
import { globalSessionsApi, type WatchedSession } from '~/lib/sessionsApi'
import { HelperPill } from '../sessions/SessionRow'

// What the registry records as the answerer when HQ itself says yes toward the goal.
const DRIVE_ANSWERER = 'a Drive turn'
const EXPLAINED_KEY = 'hq.chat.sayYesExplained'

function wasExplained() {
  try {
    return localStorage.getItem(EXPLAINED_KEY) === '1'
  } catch {
    return false
  }
}

const clock = (when: string | null) =>
  when ? parseSqliteUtc(when).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

interface Props {
  sessions: WatchedSession[]
  busy: Set<string>
  onDrive: (id: string, drive: boolean) => void
  onUnwatch: (id: string) => void
  /** Archives the chat the panel belongs to. */
  onArchive?: () => void
}

/** The details panel: the helpers this chat watches, each with its say-yes switch and answers, and what is set for the chat. */
export function SessionsPanel({ sessions, busy, onDrive, onUnwatch, onArchive }: Props) {
  const [explaining, setExplaining] = useState<string | null>(null)

  const turn = (id: string, drive: boolean) => {
    if (drive && !wasExplained()) setExplaining(id)
    else onDrive(id, drive)
  }
  const confirm = () => {
    try {
      localStorage.setItem(EXPLAINED_KEY, '1')
    } catch {
      // The explainer shows again next time; turning it on still works.
    }
    if (explaining) onDrive(explaining, true)
    setExplaining(null)
  }

  return (
    <div className="hq-chat-side">
      {sessions.length > 0 && (
        <>
          <p className="a-grp">
            {t('chat_watching')} <em className="a-count">{sessions.length}</em>
          </p>
          <ul className="hq-chat-side" style={{ padding: 0 }} aria-label={t('helpers_watched_aria')}>
            {sessions.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                busy={busy.has(s.id)}
                onDrive={(drive) => turn(s.id, drive)}
                onUnwatch={() => onUnwatch(s.id)}
              />
            ))}
          </ul>
        </>
      )}
      <p className="a-grp">{t('chat_this_chat')}</p>
      <p className="a-body-sm a-muted">{t('chat_alerts_note')}</p>
      {onArchive && (
        <div className="a-btns">
          <button type="button" onClick={onArchive} className="a-btn quiet compact">
            <Archive className="a-ic sm" aria-hidden="true" />
            {t('chat_row_archive')}
          </button>
        </div>
      )}
      {explaining && (
        <Dialog
          kind="ask"
          title={t('chat_say_yes_title')}
          body={t('chat_say_yes_body')}
          keepLabel={t('chat_say_yes_keep')}
          actLabel={t('chat_say_yes_go')}
          onKeep={() => setExplaining(null)}
          onAct={confirm}
        />
      )}
    </div>
  )
}

interface RowProps {
  session: WatchedSession
  busy: boolean
  onDrive: (drive: boolean) => void
  onUnwatch: () => void
}

function SessionRow({ session: s, busy, onDrive, onUnwatch }: RowProps) {
  const running = s.status === 'running'
  const blocked = s.agent_status === 'blocked'
  const name = `${agentName(s.harness)} · ${s.label}`
  return (
    <li className={`a-card ${blocked ? 'needs' : ''} ${running ? '' : 'is-past'}`}>
      <div className="a-card-head">
        <b>
          <a href={`/helpers/${encodeURIComponent(s.id)}`} title={s.cwd}>
            {name}
          </a>
        </b>
      </div>
      <StatusLine session={s} />
      {s.task && <TaskLine task={s.task} />}
      <GoalLines session={s} />
      {blocked && (
        <a href={`/helpers/${encodeURIComponent(s.id)}`} className="a-btn compact" style={{ alignSelf: 'flex-start' }}>
          {t('chat_answer_it')}
        </a>
      )}
      <DriveSwitch id={s.id} drive={s.drive} busy={busy} onChange={onDrive} />
      <AnswerLog sessionId={s.id} />
      {!s.drive && s.drive_off_reason && <p className="a-body-sm a-warn-t">{t('helpers_drive_off_reason', { reason: s.drive_off_reason })}</p>}
      {!s.drive && s.drive_blocked_by.length > 0 && (
        <p className="a-body-sm a-warn-t">{t('helpers_observes_until', { reasons: s.drive_blocked_by.join(', ') })}</p>
      )}
      <button
        type="button"
        onClick={onUnwatch}
        disabled={busy}
        className="a-btn quiet compact"
        style={{ alignSelf: 'flex-start' }}
        aria-label={t('helpers_unwatch_aria', { name })}
      >
        <EyeOff className="a-ic sm" aria-hidden="true" />
        {t('chat_stop_watching')}
      </button>
      <p className="a-cap a-muted">{t('chat_stop_watching_hint')}</p>
    </li>
  )
}

function TaskLine({ task }: { task: NonNullable<WatchedSession['task']> }) {
  const closeOverlay = useHQStore((st) => st.setGlobalChatOpen)
  const selectTask = useHQStore((st) => st.setSelectedTaskId)
  return (
    <div className="a-drow">
      <span>{t('helpers_ins_task')}</span>
      <span>
        <Link
          to="/tasks"
          className="a-link"
          onClick={() => {
            selectTask(task.id)
            closeOverlay(false)
          }}
          title={t('helpers_open_task')}
        >
          {task.display_id} {task.title} · {STATUS_LABELS[task.status]}
        </Link>
      </span>
    </div>
  )
}

function GoalLines({ session: s }: { session: WatchedSession }) {
  return (
    <>
      {s.goal && (
        <p className="a-body-sm">
          <b>{t('helpers_goal')}</b>
          <br />
          {s.goal}
        </p>
      )}
      {s.done_criteria && (
        <p className="a-body-sm">
          <b>{t('helpers_done_when')}</b>
          <br />
          {s.done_criteria}
        </p>
      )}
    </>
  )
}

function StatusLine({ session: s }: { session: WatchedSession }) {
  const seen = ago(s.last_seen_at)
  const driven = ago(s.last_driven_at)
  const facts = [computerName(s.host), seen && t('helpers_seen', { when: seen }), s.pending_wake && t('helpers_wake_pending', { what: s.pending_wake }), driven && t('helpers_driven', { when: driven })]
  return (
    <>
      <HelperPill session={s} />
      <span className="a-body-sm a-muted">{facts.filter(Boolean).join(' · ')}</span>
    </>
  )
}

function DriveSwitch({ id, drive, busy, onChange }: { id: string; drive: boolean; busy: boolean; onChange: (drive: boolean) => void }) {
  const labelId = `hq-drive-${id}`
  return (
    <div className="hq-swrow">
      <span>
        <b id={labelId}>{t('chat_say_yes')}</b>
        <i>{drive ? t('chat_say_yes_on') : t('chat_say_yes_off')}</i>
      </span>
      <button type="button" className="a-sw" role="switch" aria-checked={drive} aria-labelledby={labelId} aria-busy={busy || undefined} disabled={busy} onClick={() => onChange(!drive)} />
    </div>
  )
}

/** Every answer HQ gave for the helper, with its time, asked of the server's answers list. */
function AnswerLog({ sessionId }: { sessionId: string }) {
  const answers = useAnswers(sessionId)
  return (
    <div className="a-body-sm">
      <b>{tn('chat_say_yes_log', answers.length)}</b>
      {answers.length === 0 ? (
        <p className="a-muted">{t('chat_say_yes_none')}</p>
      ) : (
        <ul>
          {answers.map((a, i) => (
            <li key={`${a.at}-${i}`}>{answerLine(a)}</li>
          ))}
        </ul>
      )}
    </div>
  )
}

function useAnswers(sessionId: string): AutoAnswer[] {
  const [answers, setAnswers] = useState<AutoAnswer[]>([])
  useEffect(() => {
    let live = true
    globalSessionsApi
      .answers(sessionId)
      .then((list) => live && setAnswers(autoAnswers(list)))
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [sessionId])
  return answers
}

function answerLine(a: AutoAnswer): string {
  const time = clock(a.at)
  if (a.by !== DRIVE_ANSWERER) return t('chat_say_yes_by', { time, by: a.by, kind: a.kind })
  return t(a.declined ? 'chat_say_yes_declined' : 'chat_say_yes_allowed', { time, kind: a.kind })
}
