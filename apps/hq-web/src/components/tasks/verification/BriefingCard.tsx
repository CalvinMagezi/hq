import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { AlertTriangle, CheckCircle2, Hourglass, Undo2 } from 'lucide-react'
import { fetchBriefing, vetoTask, type Briefing } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../../sessions/usePolled'
import { countdown, levelLabel, verificationActive } from './verification'

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
  return <h3 className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1.5">{children}</h3>
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
    <section aria-label="While you were away" className="mb-4 rounded-2xl hq-glass-card px-4 py-3.5 space-y-3.5">
      <div className="flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold text-white" style={{ fontFamily: 'var(--font-heading)' }}>
          While you were away
        </h2>
        <span className="text-[11px] text-neutral-500">last {BRIEFING_HOURS} hours</span>
      </div>

      {b.mode === 'shadow' && (
        <p className="text-xs text-amber-300/90">Verification is only watching: it records what it would do and moves nothing.</p>
      )}

      {b.needs_you_total === 0 ? (
        <p className="flex items-center gap-1.5 text-xs text-emerald-300">
          <CheckCircle2 className="w-3.5 h-3.5" />
          Nothing needs you.
        </p>
      ) : b.bulk_offer ? (
        <div className="rounded-xl border border-amber-400/25 bg-amber-400/10 px-3 py-2.5 space-y-2">
          <p className="text-xs font-semibold text-amber-200">{b.needs_you_total} need you</p>
          <p className="text-xs text-amber-100/90">{b.bulk_offer.text}</p>
          {variant === 'tasks' && onPlanSweep ? (
            <button type="button" onClick={onPlanSweep} className="hq-btn-primary !h-10 !px-3.5 !text-xs">
              Plan a sweep
            </button>
          ) : (
            <p className="text-[11px] text-amber-200/80">
              Open <Link to="/tasks" className="underline">Tasks</Link> to plan it, or run <code>{b.bulk_offer.command}</code>.
            </p>
          )}
        </div>
      ) : (
        <div>
          <Heading>Needs you ({b.needs_you_total})</Heading>
          <ul className="space-y-1.5">
            {b.needs_you.map((i) => (
              <li key={i.task_id}>
                <button
                  type="button"
                  onClick={() => onOpenTask?.(i.task_id)}
                  className="w-full text-left min-h-11 px-3 py-2 rounded-xl bg-white/[0.03] border border-white/10 hover:bg-white/[0.06]"
                >
                  <span className="block text-xs text-neutral-100 truncate">
                    <span className="font-semibold">{i.display_id}</span> {i.title}
                  </span>
                  <span className="block text-[11px] text-neutral-400 truncate">{i.reason}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.veto_windows.length > 0 && (
        <div>
          <Heading>Tier 2, inside the veto window</Heading>
          <ul className="space-y-1.5">
            {b.veto_windows.map((w) => (
              <li key={w.task_id} className="flex items-center gap-2 min-h-11 px-3 py-1.5 rounded-xl bg-white/[0.03] border border-white/10">
                <Hourglass className="w-3.5 h-3.5 text-amber-300 shrink-0" />
                <span className="min-w-0 flex-1">
                  <span className="block text-xs text-neutral-200 truncate">
                    <span className="font-semibold">{w.display_id}</span> {w.title}
                  </span>
                  <span className="block text-[11px] text-amber-200/80">completes in {countdown(w.release_at, nowMs)}</span>
                </span>
                {onVeto && (
                  <button
                    type="button"
                    disabled={busyVeto === w.task_id}
                    onClick={() => onVeto(w.task_id)}
                    className="hq-btn-ghost !h-9 !px-3 !text-xs disabled:opacity-40"
                  >
                    <Undo2 className="w-3.5 h-3.5" />
                    Veto
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.completed_total > 0 && (
        <div>
          <Heading>HQ verified and completed {b.completed_total}</Heading>
          <ul className="space-y-1">
            {b.completed.map((g) => (
              <li key={g.initiative_id} className="text-xs text-neutral-300">
                <span className="font-semibold text-white">{g.count}</span> in {g.initiative || 'other'}
                <span className="text-neutral-500">
                  {' '}
                  ({Object.entries(g.by_level).map(([level, n]) => `${n} on ${levelLabel(level)}`).join(', ')}
                  {g.tier2 > 0 ? `, ${g.tier2} tier 2` : ''})
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {b.reworked_total > 0 && (
        <div>
          <Heading>Failed and sent back without you ({b.reworked_total})</Heading>
          <ul className="space-y-1">
            {b.reworked.slice(0, REWORKED_SHOWN).map((r) => (
              <li key={r.task_id} className="text-xs text-neutral-400 truncate">
                <span className="text-neutral-200">{r.display_id}</span> {r.title}
                {r.last_finding ? <span className="text-neutral-500"> · {r.last_finding}</span> : null}
              </li>
            ))}
            {b.reworked_total > REWORKED_SHOWN && <li className="text-[11px] text-neutral-500">and {b.reworked_total - REWORKED_SHOWN} more</li>}
          </ul>
        </div>
      )}

      {(b.open_audits > 0 || b.audit_pool > 0) && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <p className="text-xs text-neutral-400 mr-auto">
            {b.open_audits > 0
              ? `${b.open_audits} completed ${b.open_audits === 1 ? 'task is' : 'tasks are'} waiting for your agree or not.`
              : `${b.audit_pool} completed tasks nobody has checked yet.`}
          </p>
          {variant === 'tasks' && onAudit && (
            <button type="button" onClick={onAudit} className="hq-btn-ghost !h-9 !px-3 !text-xs">
              Audit
            </button>
          )}
        </div>
      )}

      {b.health.alerts.length > 0 && (
        <ul className="space-y-1">
          {b.health.alerts.map((a) => (
            <li key={a} className="flex items-start gap-1.5 text-xs text-amber-300">
              <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
              {a}
            </li>
          ))}
        </ul>
      )}

      {nothing && <p className="text-xs text-neutral-500">Nothing happened that needs a decision.</p>}
      {variant === 'home' && (
        <Link to="/tasks" className="inline-block text-xs text-emerald-400 hover:text-emerald-300">
          Open tasks
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
  const found = usePolled('verification-briefing', () => fetchBriefing(BRIEFING_HOURS), BRIEFING_POLL_MS)
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
