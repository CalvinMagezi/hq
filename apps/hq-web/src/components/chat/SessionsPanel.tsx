import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Archive, EyeOff, Loader2, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { agentName, computerName } from '~/lib/names'
import { t, tn } from '~/strings'
import { autoAnswers, type AutoAnswer } from '~/lib/autoAnswers'
import { Dialog } from '../ui/Dialog'
import type { WatchedSession } from '~/lib/sessionsApi'
import { STATUS_BADGE_CLASS } from '../tasks/TaskCard'
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
    <div className="border-b border-white/10 bg-neutral-900/60 shrink-0 max-h-[50dvh] overflow-y-auto overscroll-contain">
      <h2 className="max-w-4xl mx-auto px-3 sm:px-4 pt-2 text-xs font-semibold">
        {t('chat_details')} · {t('chat_watching_count', { n: sessions.length })}
      </h2>
      <ul className="max-w-4xl mx-auto" aria-label={t('helpers_watched_aria')}>
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
      <section className="max-w-4xl mx-auto px-3 sm:px-4 py-2.5 text-xs border-t border-white/5">
        <h3 className="font-semibold">{t('chat_this_chat')}</h3>
        <p className="mt-1 text-neutral-500">{t('chat_alerts_note')}</p>
        {onArchive && (
          <button type="button" onClick={onArchive} className="mt-1.5 flex items-center gap-1.5 h-8 px-2 rounded border border-white/10 text-neutral-300 hover:text-white hover:bg-white/5">
            <Archive className="w-3.5 h-3.5" aria-hidden="true" /> {t('chat_row_archive')}
          </button>
        )}
      </section>
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
  return (
    <li className={`px-3 sm:px-4 py-2.5 text-xs border-t border-white/5 first:border-t-0 ${running ? '' : 'opacity-50'}`}>
      <div className="flex items-center gap-2 min-w-0">
        <Terminal className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
        <span className="text-neutral-200 truncate min-w-0" title={s.cwd}>
          {agentName(s.harness)} · {s.label}
        </span>
        <span className="hidden sm:inline text-neutral-500 truncate min-w-0">{computerName(s.host)}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onUnwatch}
          disabled={busy}
          className="flex items-center justify-center gap-1 h-11 min-w-11 sm:h-8 sm:min-w-0 px-2 rounded text-neutral-400 hover:text-white hover:bg-white/10 disabled:opacity-50 shrink-0"
          title={t('chat_stop_watching_hint')}
          aria-label={t('helpers_unwatch_aria', { name: `${agentName(s.harness)} ${s.label}` })}
        >
          <EyeOff className="w-3.5 h-3.5" />
          <span className="hidden sm:inline">{t('helpers_unwatch')}</span>
        </button>
      </div>
      {s.task && <TaskLine task={s.task} />}
      <GoalLines session={s} />
      <StatusLine session={s} />
      <DriveSwitch drive={s.drive} busy={busy} onChange={onDrive} />
      <AnswerLog answers={autoAnswers(s.events)} />
      {!s.drive && s.drive_off_reason && (
        <p className="mt-1 text-xs" style={{ color: 'var(--warn)' }}>
          {t('helpers_drive_off_reason', { reason: s.drive_off_reason })}
        </p>
      )}
      {!s.drive && s.drive_blocked_by.length > 0 && (
        <p className="mt-1 text-xs" style={{ color: 'var(--warn)' }}>
          {t('helpers_observes_until', { reasons: s.drive_blocked_by.join(', ') })}
        </p>
      )}
    </li>
  )
}

function TaskLine({ task }: { task: NonNullable<WatchedSession['task']> }) {
  const closeOverlay = useHQStore((st) => st.setGlobalChatOpen)
  const selectTask = useHQStore((st) => st.setSelectedTaskId)
  return (
    <Link
      to="/tasks"
      onClick={() => {
        selectTask(task.id)
        closeOverlay(false)
      }}
      className="mt-1.5 flex items-center gap-2 min-w-0 text-neutral-300 hover:text-white"
      title={t('helpers_open_task')}
    >
      <span className="text-neutral-500 shrink-0">{task.display_id}</span>
      <span className="min-w-0 line-clamp-2 sm:line-clamp-1">{task.title}</span>
      <span className={`shrink-0 text-xs px-2 py-0.5 rounded-full border ${STATUS_BADGE_CLASS[task.status]}`}>
        {STATUS_LABELS[task.status]}
      </span>
    </Link>
  )
}

function GoalLines({ session: s }: { session: WatchedSession }) {
  if (!s.goal && !s.done_criteria) return null
  return (
    <dl className="mt-1 text-neutral-400 space-y-0.5">
      {s.goal && (
        <div className="flex gap-2 min-w-0">
          <dt className="text-neutral-500 shrink-0">{t('helpers_goal')}</dt>
          <dd className="min-w-0 line-clamp-2">{s.goal}</dd>
        </div>
      )}
      {s.done_criteria && (
        <div className="flex gap-2 min-w-0">
          <dt className="text-neutral-500 shrink-0">{t('helpers_done_when')}</dt>
          <dd className="min-w-0 line-clamp-2">{s.done_criteria}</dd>
        </div>
      )}
    </dl>
  )
}

function StatusLine({ session: s }: { session: WatchedSession }) {
  const seen = ago(s.last_seen_at)
  const driven = ago(s.last_driven_at)
  return (
    <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-neutral-500">
      <span className="sm:hidden">{computerName(s.host)}</span>
      <HelperPill session={s} />
      {seen && <span>{t('helpers_seen', { when: seen })}</span>}
      {s.pending_wake && <span>{t('helpers_wake_pending', { what: s.pending_wake })}</span>}
      {driven && <span>{t('helpers_driven', { when: driven })}</span>}
    </div>
  )
}

function DriveSwitch({ drive, busy, onChange }: { drive: boolean; busy: boolean; onChange: (drive: boolean) => void }) {
  return (
    <div className="mt-1.5 flex items-center gap-2 min-w-0">
      <button
        type="button"
        role="switch"
        aria-checked={drive}
        onClick={() => onChange(!drive)}
        disabled={busy}
        className={`flex items-center gap-1.5 h-11 sm:h-8 px-3 sm:px-2 rounded border shrink-0 disabled:opacity-50 ${
          drive ? 'hq-chip-on' : 'border-white/10 text-neutral-400 hover:text-white hover:bg-white/5'
        }`}
        title={drive ? t('chat_say_yes_on') : t('chat_say_yes_off')}
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <span className={`w-2 h-2 rounded-full ${drive ? 'bg-white' : 'bg-neutral-500'}`} />}
        {t('chat_say_yes')} · {drive ? t('helpers_drive_on') : t('helpers_drive_off')}
      </button>
      <span className="hidden sm:inline text-xs text-neutral-500">{drive ? t('chat_say_yes_on') : t('chat_say_yes_off')}</span>
    </div>
  )
}

/** Every answer HQ gave for the helper, with its time. Empty until the server lists the helper's events. */
function AnswerLog({ answers }: { answers: AutoAnswer[] }) {
  return (
    <div className="mt-1.5 text-neutral-400">
      <p className="font-semibold text-neutral-300">{tn('chat_say_yes_log', answers.length)}</p>
      {answers.length === 0 ? (
        <p className="text-neutral-500">{t('chat_say_yes_none')}</p>
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

function answerLine(a: AutoAnswer): string {
  const time = clock(a.at)
  if (a.by !== DRIVE_ANSWERER) return t('chat_say_yes_by', { time, by: a.by, kind: a.kind })
  return t(a.declined ? 'chat_say_yes_declined' : 'chat_say_yes_allowed', { time, kind: a.kind })
}
