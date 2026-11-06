import { t, tn } from '~/strings'
import '../leaf.css'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { AlertTriangle, CheckCircle2, Hourglass, Undo2 } from 'lucide-react'
import { fetchBriefing, vetoTask, type Briefing } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../../sessions/usePolled'
import { countdown, kindLabel, levelLabel, verificationActive } from './verification'

const BRIEFING_POLL_MS = 120_000
const BRIEFING_HOURS = 24
/** Tasks named under "failed and reworked"; the count still covers all of them. */
const REWORKED_SHOWN = 3

interface ViewProps {
  briefing: Briefing
  nowMs: number
  /** The tasks page has the sheets; the home screen only links there. */
  variant: 'tasks' | 'home'
  busyVeto?: string | null
  onOpenTask?: (id: string) => void
  onVeto?: (taskId: string) => void
  onPlanSweep?: () => void
  onAudit?: () => void
}

function Heading({ children }: { children: React.ReactNode }) {
  return <h3 className="a-label mb-2">{children}</h3>
}

/**
 * "While you were away", exception first: what needs the owner, then what was verified and
 * completed (a line per initiative, never a pass at a time), what failed and was reworked without
 * them, tier 2 tasks inside their veto window, and the audit prompt. Past the threshold the list
 * of what needs them is replaced by one offer, because a long flat list is the bottleneck.
 */
export function BriefingCardView({ briefing: b, nowMs, variant, busyVeto, onOpenTask, onVeto, onPlanSweep, onAudit }: ViewProps) {
  const nothing =
    b.needs_you_total === 0 && b.completed_total === 0 && b.reworked_total === 0 && b.veto_windows.length === 0 && b.open_audits === 0 && b.health.alerts.length === 0
  return (
    <section aria-label="While you were away" className="a-card mb-4">
      <div className="l-row justify-between">
        <h2 className="a-h3">
          {t('tasks_away_title')}
        </h2>
        <span className="a-cap a-muted">{t('tasks_away_window', { hours: BRIEFING_HOURS })}</span>
      </div>

      {b.mode === 'shadow' && (
        <p className="a-why"><AlertTriangle className="a-ic xs" aria-hidden="true" />{t('check_watching_only_line')}</p>
      )}

      {b.needs_you_total === 0 ? (
        <p className="l-row a-body-sm a-muted">
          <CheckCircle2 className="a-ic xs" aria-hidden="true" />
          {t('tasks_away_nothing')}
        </p>
      ) : b.bulk_offer ? (
        <div className="a-card needs">
          <p className="a-body-sm-b a-warn-t">{t('check_needs_you_count', { n: b.needs_you_total })}</p>
          <p className="a-body-sm">{b.bulk_offer.text}</p>
          {variant === 'tasks' && onPlanSweep ? (
            <button type="button" onClick={onPlanSweep} className="a-btn compact">
              {t('check_plan_sweep')}
            </button>
          ) : (
            <p className="a-body-sm a-muted">
              Open <Link to="/tasks" className="a-link">Tasks</Link> to plan it, or run <code>{b.bulk_offer.command}</code>.
            </p>
          )}
        </div>
      ) : (
        <div className="l-sec">
          <Heading>{t('check_needs_you_head', { n: b.needs_you_total })}</Heading>
          <ul className="a-card l-lines">
            {b.needs_you.map((i) => (
              <li key={i.task_id}>
                <button
                  type="button"
                  onClick={() => onOpenTask?.(i.task_id)}
                  className="a-drow l-tall l-col l-link w-full"
                >
                  <span className="block a-body-sm l-clip">
                    <b>{i.display_id}</b> {i.title}
                  </span>
                  <span className="block a-cap a-muted l-clip">{i.reason}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.veto_windows.length > 0 && (
        <div className="l-sec">
          <Heading>{t('tasks_away_finishing')}</Heading>
          <ul className="a-card l-lines">
            {b.veto_windows.map((w) => (
              <li key={w.task_id} className="a-drow l-tall">
                <Hourglass className="a-ic xs a-warn-t" aria-hidden="true" />
                <span className="l-grow">
                  <span className="block a-body-sm l-clip">
                    <b>{w.display_id}</b> {w.title}
                  </span>
                  <span className="block a-cap a-warn-t">{t('tasks_away_finishes_at', { time: countdown(w.release_at, nowMs) })}</span>
                </span>
                {onVeto && (
                  <button
                    type="button"
                    disabled={busyVeto === w.task_id}
                    onClick={() => onVeto(w.task_id)}
                    className="a-btn secondary compact"
                  >
                    <Undo2 className="a-ic xs" aria-hidden="true" />
                    {t('check_keep_open')}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.completed_total > 0 && (
        <div className="l-sec">
          <Heading>{t('tasks_away_finished')} {b.completed_total}</Heading>
          <ul className="l-stack">
            {b.completed.map((g) => (
              <li key={g.initiative_id} className="a-body-sm">
                <b>{g.count}</b> in {g.initiative || t('check_other_area')}
                <span className="a-muted">
                  {' '}
                  ({Object.entries(g.by_level).map(([level, n]) => `${n} on ${levelLabel(level)}`).join(', ')}
                  {g.tier2 > 0 ? `, ${g.tier2} ${kindLabel(2).toLowerCase()}` : ''})
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.reworked_total > 0 && (
        <div className="l-sec">
          <Heading>{t('check_rework_head', { n: b.reworked_total })}</Heading>
          <ul className="l-stack">
            {b.reworked.slice(0, REWORKED_SHOWN).map((r) => (
              <li key={r.task_id} className="a-body-sm a-muted l-clip">
                <b>{r.display_id}</b> {r.title}
                {r.last_finding ? <span className="a-muted"> · {r.last_finding}</span> : null}
              </li>
            ))}
            {b.reworked_total > REWORKED_SHOWN && <li className="a-cap a-muted">and {b.reworked_total - REWORKED_SHOWN} more</li>}
          </ul>
        </div>
      )}

      {(b.open_audits > 0 || b.audit_pool > 0) && (
        <div className="l-bar">
          <p className="a-body-sm a-muted l-grow">
            {b.open_audits > 0
              ? tn('briefing_audits_waiting', b.open_audits)
              : `${b.audit_pool} completed tasks nobody has checked yet.`}
          </p>
          {variant === 'tasks' && onAudit && (
            <button type="button" onClick={onAudit} className="a-btn secondary compact">
              {t('check_audit')}
            </button>
          )}
        </div>
      )}

      {b.health.alerts.length > 0 && (
        <ul className="l-stack">
          {b.health.alerts.map((a) => (
            <li key={a} className="a-why">
              <AlertTriangle className="a-ic xs" aria-hidden="true" />
              {a}
            </li>
          ))}
        </ul>
      )}

      {nothing && <p className="a-body-sm a-muted">{t('tasks_away_nothing')}</p>}
      {variant === 'home' && (
        <Link to="/tasks" className="a-link">
          {t('tasks_away_open_tasks')}
        </Link>
      )}
    </section>
  )
}

interface Props {
  variant: 'tasks' | 'home'
  onOpenTask?: (id: string) => void
  onPlanSweep?: () => void
  onAudit?: () => void
}

/** Polls the briefing; hidden while verification is off, and (at home) while there is nothing to say. */
export function BriefingCard({ variant, onOpenTask, onPlanSweep, onAudit }: Props) {
  const found = usePolled('verification-briefing', () => fetchBriefing(BRIEFING_HOURS), BRIEFING_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  const [busyVeto, setBusyVeto] = useState<string | null>(null)
  const b = found.data
  if (!b || !verificationActive(b.mode)) return null
  const quiet = b.needs_you_total === 0 && b.completed_total === 0 && b.reworked_total === 0 && b.veto_windows.length === 0 && b.open_audits === 0 && b.health.alerts.length === 0
  if (variant === 'home' && quiet) return null

  const veto = async (taskId: string) => {
    setBusyVeto(taskId)
    try {
      await vetoTask(taskId, 'vetoed from the briefing')
      await found.refresh()
    } finally {
      setBusyVeto(null)
    }
  }
  return (
    <BriefingCardView
      briefing={b}
      nowMs={Date.now()}
      variant={variant}
      busyVeto={busyVeto}
      onOpenTask={onOpenTask}
      onVeto={variant === 'tasks' ? veto : undefined}
      onPlanSweep={onPlanSweep}
      onAudit={onAudit}
    />
  )
}
