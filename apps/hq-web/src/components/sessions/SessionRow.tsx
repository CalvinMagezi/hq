import { Link } from '@tanstack/react-router'
import { ArchiveRestore, Hand, Loader2, Terminal, WifiOff } from 'lucide-react'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { AgentStatus, HarnessSession } from '~/lib/sessionsApi'
import { t } from '~/strings'
import { computerName, folderName, sessionTitle } from '~/lib/workbench'
import { cardFacts } from '~/lib/helperView'
import { InlineAnswer } from './HelperAnswer'
import { STATUS_BADGE_CLASS } from '../tasks/TaskCard'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

export type HelperState = 'needs' | 'working' | 'ready' | 'offline' | 'lost' | 'finished' | 'stopped'

interface StateInput {
  status: string
  agent_status: AgentStatus | null
  alive?: boolean | null
  reachable?: boolean
}

/** One state per helper, whichever list or panel shows it. */
export function helperState(s: StateInput): HelperState {
  if (s.reachable === false) return 'offline'
  if (s.status === 'running') {
    if (s.alive === false) return 'lost'
    if (s.agent_status === 'blocked') return 'needs'
    if (s.agent_status === 'idle') return 'ready'
    if (s.agent_status === 'done') return 'finished'
    return 'working'
  }
  if (s.status === 'exited') return 'finished'
  if (s.status === 'orphaned') return 'lost'
  return 'stopped'
}

const PILL_CLASS: Record<HelperState, string> = {
  needs: 'needs',
  working: 'working',
  ready: 'ready',
  offline: 'offline',
  lost: 'offline',
  finished: '',
  stopped: '',
}

const PILL_WORDS = {
  needs: 'helper_state_needs',
  working: 'helper_state_working',
  ready: 'helper_state_ready',
  offline: 'helper_state_offline',
  lost: 'helper_state_lost',
  finished: 'helper_state_finished',
  stopped: 'helper_state_stopped',
} as const

/** The state pill: amber needs you or lost, blue working, ink ready, grey done. The words always say it. */
export function HelperPill({ session }: { session: StateInput }) {
  const state = helperState(session)
  return (
    <span className={`a-state ${PILL_CLASS[state]}`} data-state={state}>
      {state === 'needs' && <Hand className="a-ic" aria-hidden="true" />}
      {state === 'working' && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
      {(state === 'offline' || state === 'lost') && <WifiOff className="a-ic" aria-hidden="true" />}
      {t(PILL_WORDS[state])}
    </span>
  )
}

/** Friendly state pill, computer and last-seen time, shared by the list row and the detail header. */
export function SessionBadges({ session: s }: { session: HarnessSession }) {
  const seen = ago(s.last_seen_at)
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-400">
      <HelperPill session={s} />
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
      className="flex items-center gap-2 min-w-0 text-xs text-neutral-300 hover:text-white"
      title={t('helpers_open_task')}
    >
      <span className="text-neutral-500 shrink-0">{task.display_id}</span>
      <span className="min-w-0 truncate">{task.title}</span>
      <span className={`shrink-0 text-xs px-2 py-0.5 rounded-full border ${STATUS_BADGE_CLASS[task.status]}`}>
        {STATUS_LABELS[task.status]}
      </span>
    </Link>
  )
}

interface Props {
  session: HarnessSession
  selected: boolean
  /** Called after an in-place answer, so the list reloads. */
  onAnswered: () => unknown
  /** Offered on an archived card, beside its link. */
  onRestore?: () => void
  busy?: boolean
  /** Why answering is off while HQ can't be reached. */
  answersOff?: string | null
}

/** One helper in the list: a title link to its page, one line of facts, and for a plain question the answer in place. */
export function SessionRow({ session: s, selected, onAnswered, onRestore, busy, answersOff = null }: Props) {
  const folder = folderName(s.cwd)
  const state = helperState(s)
  return (
    <li className={`hq-row mb-1.5 ${selected ? 'is-selected' : ''} ${s.status === 'running' ? '' : 'opacity-70'}`}>
      <div className="flex items-stretch">
        <Link
          to="/helpers/$id"
          params={{ id: s.id }}
          search={(prev: Record<string, unknown>) => ({ ...prev, id: undefined })}
          aria-current={selected ? 'true' : undefined}
          className="flex-1 min-w-0 text-left px-3.5 py-3 space-y-1 rounded-xl min-h-11 block"
        >
          <span className="flex items-center gap-2 min-w-0 text-sm font-semibold">
            <Terminal className="w-3.5 h-3.5 text-neutral-500 shrink-0" aria-hidden="true" />
            <span className="text-neutral-100 truncate min-w-0">{sessionTitle(s)}</span>
          </span>
          {folder && <span className="block text-xs text-neutral-400 truncate">{folder}</span>}
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-neutral-400">
            <HelperPill session={s} />
            {cardFacts(s).map((fact, i) => (
              <span key={i} className="min-w-0 break-words">
                {fact}
              </span>
            ))}
          </span>
        </Link>
        {onRestore && (
          <button type="button" onClick={onRestore} disabled={busy} aria-label={t('helpers_restore_aria', { helper: sessionTitle(s) })} className="a-btn secondary self-center mr-2 shrink-0">
            <ArchiveRestore className="w-4 h-4" aria-hidden="true" />
            {t('helpers_restore')}
          </button>
        )}
      </div>
      {s.task && (
        <div className="px-3.5 pb-2.5">
          <SessionTaskLink session={s} />
        </div>
      )}
      {state === 'needs' && !s.archived && <InlineAnswer session={s} onDone={onAnswered} off={answersOff} />}
    </li>
  )
}
