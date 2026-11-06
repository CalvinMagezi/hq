import { Link } from '@tanstack/react-router'
import { ArchiveRestore, Hand, Loader2, WifiOff } from 'lucide-react'
import { STATUS_LABELS, parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { AgentStatus, HarnessSession } from '~/lib/sessionsApi'
import { t } from '~/strings'
import { computerName, sessionTitle } from '~/lib/workbench'
import { cardFacts } from '~/lib/helperView'
import { InlineAnswer } from './HelperAnswer'

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
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 a-cap a-muted">
      <HelperPill session={s} />
      <span>{computerName(s.host)}</span>
      {seen && <span>{t('helpers_seen', { when: seen })}</span>}
    </div>
  )
}

/** The linked task with a link into the Tasks page, or nothing for an ad hoc agent. */
export function SessionTaskLink({ session: s }: { session: HarnessSession }) {
  const task = s.task
  if (!task) return null
  return (
    <Link to="/tasks" search={{ task: task.id }} className="a-link inline-flex items-center gap-2 min-w-0" title={t('helpers_open_task')}>
      <span className="shrink-0">{task.display_id}</span>
      <span className="min-w-0 truncate">{task.title}</span>
      <span className="a-state">{STATUS_LABELS[task.status]}</span>
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

const EDGE_STATES: readonly HelperState[] = ['needs', 'offline', 'lost']

/** One helper in the list: the title is the link to its page, the state pill beside it, the facts under it, and for a plain question the answer in place. */
export function SessionRow({ session: s, selected, onAnswered, onRestore, busy, answersOff = null }: Props) {
  const state = helperState(s)
  const [kind, where, span, doing] = cardFacts(s)
  return (
    <li>
      <div className={`a-card ${EDGE_STATES.includes(state) && !s.archived ? 'needs' : ''} ${selected ? 'is-sel' : ''}`}>
        <div className="a-card-head">
          <Link
            to="/helpers/$id"
            params={{ id: s.id }}
            search={(prev: Record<string, unknown>) => ({ ...prev, id: undefined })}
            aria-current={selected ? 'true' : undefined}
            className="flex-1 min-w-0"
          >
            <b className="a-body-b break-words">{sessionTitle(s)}</b>
          </Link>
          <HelperPill session={s} />
        </div>
        <span className="a-body-sm a-muted">{[kind, where, span].filter(Boolean).join(' · ')}</span>
        <span className="a-body-sm">{doing}</span>
        {state === 'working' && <span className="a-work" aria-hidden="true" />}
        {s.task && <SessionTaskLink session={s} />}
        {onRestore && (
          <div className="a-card-actions">
            <button type="button" onClick={onRestore} disabled={busy} aria-label={t('helpers_restore_aria', { helper: sessionTitle(s) })} className="a-btn secondary">
              <ArchiveRestore className="a-ic" aria-hidden="true" />
              {t('helpers_restore')}
            </button>
          </div>
        )}
        {state === 'needs' && !s.archived && <InlineAnswer session={s} onDone={onAnswered} off={answersOff} />}
      </div>
    </li>
  )
}
