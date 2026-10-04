import { Link } from '@tanstack/react-router'
import { Terminal } from 'lucide-react'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { AGENT_STATUS_CLASS } from '../chat/SessionsPanel'
import { STATUS_BADGE_CLASS } from '../tasks/TaskCard'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

/** Status, host and reachability chips shared by the list row and the detail header. */
export function SessionBadges({ session: s }: { session: HarnessSession }) {
  const seen = ago(s.last_seen_at)
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] font-mono text-neutral-500">
      <span>{s.host}</span>
      {s.reachable === false && (
        <span style={{ color: 'var(--accent-amber)' }} title={s.detail}>
          host unreachable
        </span>
      )}
      {s.status !== 'running' && <span style={{ color: 'var(--accent-amber)' }}>{s.status}</span>}
      {s.status === 'running' && s.reachable !== false && s.agent_status && (
        <span className={AGENT_STATUS_CLASS[s.agent_status]}>{s.agent_status}</span>
      )}
      {s.alive === false && s.status === 'running' && <span>agent gone</span>}
      {seen && <span>seen {seen}</span>}
    </div>
  )
}

/** The linked task with a link into the Tasks page, or nothing for an ad hoc session. */
export function SessionTaskLink({ session: s }: { session: HarnessSession }) {
  const task = s.task
  if (!task) return null
  return (
    <Link
      to="/tasks"
      search={{ task: task.id }}
      className="flex items-center gap-2 min-w-0 text-[11px] font-mono text-neutral-300 hover:text-white"
      title="Open the task"
    >
      <span className="text-neutral-500 shrink-0">{task.display_id}</span>
      <span className="min-w-0 truncate">{task.title}</span>
      <span className={`shrink-0 text-[10px] px-2 py-0.5 rounded-full border ${STATUS_BADGE_CLASS[task.status]}`}>
        {STATUS_LABELS[task.status]}
      </span>
    </Link>
  )
}

interface Props {
  session: HarnessSession
  selected: boolean
  onSelect: () => void
}

/** One line group in the sessions list. The task link sits beside the select button, not inside it. */
export function SessionRow({ session: s, selected, onSelect }: Props) {
  return (
    <li className={`border-t border-white/5 first:border-t-0 ${selected ? 'bg-white/5' : ''} ${s.status === 'running' ? '' : 'opacity-60'}`}>
      <button
        type="button"
        onClick={onSelect}
        aria-current={selected ? 'true' : undefined}
        className="w-full text-left px-3 py-2.5 space-y-1 hover:bg-white/5 min-h-11"
      >
        <div className="flex items-center gap-2 min-w-0 text-xs font-mono">
          <Terminal className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
          <span className="text-neutral-200 truncate min-w-0">
            {s.harness} · {s.label || s.id}
          </span>
        </div>
        <SessionBadges session={s} />
        {s.goal && <p className="text-[11px] font-mono text-neutral-400 line-clamp-2">{s.goal}</p>}
      </button>
      {s.task && (
        <div className="px-3 pb-2">
          <SessionTaskLink session={s} />
        </div>
      )}
    </li>
  )
}
