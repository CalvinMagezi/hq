import { Link } from '@tanstack/react-router'
import { EyeOff, Loader2, Terminal } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { AgentStatus, WatchedSession } from '~/lib/sessionsApi'
import { STATUS_BADGE_CLASS } from '../tasks/TaskCard'

export const AGENT_STATUS_CLASS: Record<AgentStatus, string> = {
  idle: 'text-neutral-400',
  working: 'text-blue-400',
  blocked: 'text-rose-400',
  done: 'text-emerald-400',
}

const DRIVE_HINT = "HQ answers the agent and approves its prompts toward the task's goal"

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

interface Props {
  sessions: WatchedSession[]
  busy: Set<string>
  onDrive: (id: string, drive: boolean) => void
  onUnwatch: (id: string) => void
}

/** The coding-agent sessions this chat watches, each with a drive switch and an unwatch action. */
export function SessionsPanel({ sessions, busy, onDrive, onUnwatch }: Props) {
  return (
    <div className="border-b border-white/10 bg-neutral-900/60 shrink-0 max-h-[50dvh] overflow-y-auto overscroll-contain">
      <p className="sm:hidden px-3 pt-2 text-[11px] text-neutral-500">Drive: {DRIVE_HINT}.</p>
      <ul className="max-w-4xl mx-auto" aria-label="Watched coding-agent sessions">
        {sessions.map((s) => (
          <SessionRow
            key={s.id}
            session={s}
            busy={busy.has(s.id)}
            onDrive={(drive) => onDrive(s.id, drive)}
            onUnwatch={() => onUnwatch(s.id)}
          />
        ))}
      </ul>
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
          {s.harness} · {s.label}
        </span>
        <span className="hidden sm:inline text-neutral-500 truncate min-w-0">{s.host}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onUnwatch}
          disabled={busy}
          className="flex items-center justify-center gap-1 h-11 min-w-11 sm:h-8 sm:min-w-0 px-2 rounded text-neutral-400 hover:text-white hover:bg-white/10 disabled:opacity-50 shrink-0"
          title="Stop posting this session's updates here. The session keeps running."
          aria-label={`Unwatch ${s.harness} ${s.label}`}
        >
          <EyeOff className="w-3.5 h-3.5" />
          <span className="hidden sm:inline">Unwatch</span>
        </button>
      </div>
      {s.task && <TaskLine task={s.task} />}
      <GoalLines session={s} />
      <StatusLine session={s} />
      <DriveSwitch drive={s.drive} busy={busy} onChange={onDrive} />
      {!s.drive && s.drive_off_reason && (
        <p className="mt-1 text-[11px]" style={{ color: 'var(--accent-amber)' }}>
          Drive switched off: {s.drive_off_reason}
        </p>
      )}
      {!s.drive && s.drive_blocked_by.length > 0 && (
        <p className="mt-1 text-[11px]" style={{ color: 'var(--accent-amber)' }}>
          HQ only observes until: {s.drive_blocked_by.join('; ')}.
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
      title="Open the task"
    >
      <span className="text-neutral-500 shrink-0">{task.display_id}</span>
      <span className="min-w-0 line-clamp-2 sm:line-clamp-1">{task.title}</span>
      <span className={`shrink-0 text-[11px] px-2 py-0.5 rounded-full border ${STATUS_BADGE_CLASS[task.status]}`}>
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
          <dt className="text-neutral-500 shrink-0">Goal</dt>
          <dd className="min-w-0 line-clamp-2">{s.goal}</dd>
        </div>
      )}
      {s.done_criteria && (
        <div className="flex gap-2 min-w-0">
          <dt className="text-neutral-500 shrink-0">Done when</dt>
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
      <span className="sm:hidden">{s.host}</span>
      {s.status !== 'running' && <span style={{ color: 'var(--accent-amber)' }}>{s.status}</span>}
      {s.agent_status && <span className={AGENT_STATUS_CLASS[s.agent_status]}>{s.agent_status}</span>}
      {seen && <span>seen {seen}</span>}
      {s.pending_wake && <span>wake pending: {s.pending_wake}</span>}
      {driven && <span>driven {driven}</span>}
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
          drive ? 'border-emerald-400/60 text-emerald-400 bg-white/5' : 'border-white/10 text-neutral-400 hover:text-white hover:bg-white/5'
        }`}
        title={DRIVE_HINT}
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <span className={`w-2 h-2 rounded-full ${drive ? 'bg-emerald-400' : 'bg-neutral-500'}`} />}
        Drive {drive ? 'on' : 'off'}
      </button>
      <span className="hidden sm:inline text-[11px] text-neutral-500">{DRIVE_HINT}</span>
    </div>
  )
}
