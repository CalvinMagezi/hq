import { Link } from '@tanstack/react-router'
import { Archive, ArchiveRestore, Terminal } from 'lucide-react'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { computerName, folderName, sessionTitle, statusInfo } from '~/lib/workbench'
import { AGENT_STATUS_CLASS } from '../chat/SessionsPanel'
import { STATUS_BADGE_CLASS } from '../tasks/TaskCard'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

/** Friendly status word, computer and last-seen time, shared by the list row and the detail header. */
export function SessionBadges({ session: s }: { session: HarnessSession }) {
  const seen = ago(s.last_seen_at)
  const status = statusInfo(s)
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] font-mono text-neutral-500">
      <span className={status.agent ? AGENT_STATUS_CLASS[status.agent] : undefined} style={status.warn ? { color: 'var(--accent-amber)' } : undefined} title={s.detail}>
        {status.word}
      </span>
      <span>{computerName(s.host)}</span>
      {seen && <span>seen {seen}</span>}
    </div>
  )
}

/** The linked task with a link into the Tasks page, or nothing for an ad hoc agent. */
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
  /** Offered only for a past agent; archived ones offer Restore instead. */
  onArchive?: (archived: boolean) => void
  busy?: boolean
}

/** One agent in the list. The task link and archive button sit beside the select button, not inside it. */
export function SessionRow({ session: s, selected, onSelect, onArchive, busy }: Props) {
  const folder = folderName(s.cwd)
  return (
    <li className={`border-t border-white/5 first:border-t-0 ${selected ? 'bg-white/5' : ''} ${s.status === 'running' ? '' : 'opacity-70'}`}>
      <div className="flex items-stretch">
        <button
          type="button"
          onClick={onSelect}
          aria-current={selected ? 'true' : undefined}
          className="flex-1 min-w-0 text-left px-3 py-2.5 space-y-1 hover:bg-white/5 min-h-11"
        >
          <div className="flex items-center gap-2 min-w-0 text-xs font-mono">
            <Terminal className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
            <span className="text-neutral-200 truncate min-w-0">{sessionTitle(s)}</span>
          </div>
          {folder && <p className="text-[11px] font-mono text-neutral-500 truncate">{folder}</p>}
          <SessionBadges session={s} />
          {s.goal && <p className="text-[11px] font-mono text-neutral-400 line-clamp-2">{s.goal}</p>}
        </button>
        {onArchive && (
          <button
            type="button"
            onClick={() => onArchive(!s.archived)}
            disabled={busy}
            aria-label={s.archived ? `Restore ${sessionTitle(s)}` : `Archive ${sessionTitle(s)}`}
            title={s.archived ? 'Restore' : 'Archive'}
            className="flex items-center justify-center w-11 shrink-0 text-neutral-500 hover:text-white hover:bg-white/10 disabled:opacity-40"
          >
            {s.archived ? <ArchiveRestore className="w-4 h-4" /> : <Archive className="w-4 h-4" />}
          </button>
        )}
      </div>
      {s.task && (
        <div className="px-3 pb-2">
          <SessionTaskLink session={s} />
        </div>
      )}
    </li>
  )
}
