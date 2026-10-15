import { Loader2, X } from 'lucide-react'
import { fetchInitiativeActivityClient, type InitiativeActivity } from '~/lib/taskActivityApi'
import { STATUS_LABELS, type TaskStatus } from '~/lib/tasksApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { ageLabel, authorLabel, briefingIsEmpty, freshnessLines, reviewBadge, reviewSummary } from './activity'

const BRIEFING_POLL_MS = 120_000

const statusLabel = (status: string) => STATUS_LABELS[status as TaskStatus] ?? status

/** The briefing for one initiative. Presentational, like the task panel. */
export function InitiativeBriefingView({ view, nowMs }: { view: InitiativeActivity; nowMs?: number }) {
  if (briefingIsEmpty(view)) {
    return <p className="text-xs text-neutral-400">{view.initiative.name} has no active tasks, so there is nothing to brief.</p>
  }
  const counts = Object.entries(view.status_counts)
  return (
    <div className="space-y-3" data-scope="initiative">
      <p className="text-xs text-neutral-300">
        {counts.map(([status, n]) => `${n} ${statusLabel(status).toLowerCase()}`).join(', ')}
        {view.tasks_shown > 0 ? ` (${view.tasks_shown} tasks)` : ''}
      </p>

      {view.awaiting_review.length > 0 && (
        <div>
          <div className="text-[11px] uppercase tracking-wider text-neutral-500">Waiting for review</div>
          <ul className="text-xs text-neutral-200">
            {view.awaiting_review.map((t) => (
              <li key={t.id}>
                {t.display_id} {t.title}
              </li>
            ))}
          </ul>
        </div>
      )}

      {view.blocked.length > 0 && (
        <div>
          <div className="text-[11px] uppercase tracking-wider text-neutral-500">Blocked (as recorded)</div>
          <ul className="text-xs text-neutral-200">
            {view.blocked.map((t) => (
              <li key={t.id} className="break-words">
                {t.display_id} {t.title}
                {t.reason ? <span className="text-neutral-500">: {t.reason}</span> : null}
                {t.waiting_on ? <span className="text-neutral-500"> (waiting on {t.waiting_on})</span> : null}
              </li>
            ))}
          </ul>
        </div>
      )}

      {view.contributors.length > 0 ? (
        <div>
          <div className="text-[11px] uppercase tracking-wider text-neutral-500">Contributors</div>
          <ul className="text-xs text-neutral-200">
            {view.contributors.map((c) => (
              <li key={c.actor}>
                {c.actor}{' '}
                <span className="text-neutral-500">
                  {c.notes} {c.notes === 1 ? 'note' : 'notes'}
                  {c.leases > 0 ? `, ${c.leases} ${c.leases === 1 ? 'lease' : 'leases'}` : ''}
                  {c.last_active ? `, last ${ageLabel(c.last_active, nowMs)}` : ''}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p className="text-xs text-neutral-400">No one is on record as having written to these tasks.</p>
      )}

      <p className="text-xs text-neutral-300">
        Notes: {reviewSummary(view.review_counts)}. {view.gaps_total} {view.gaps_total === 1 ? 'gap' : 'gaps'} in attribution or links.
      </p>

      {view.recent_notes.length > 0 && (
        <div>
          <div className="text-[11px] uppercase tracking-wider text-neutral-500 mb-1">Recent notes</div>
          <ul className="space-y-1">
            {view.recent_notes.map((n) => {
              const badge = reviewBadge(n)
              return (
                <li key={`${n.task_display_id}-${n.path}`} className="text-xs text-neutral-200 flex items-start gap-2">
                  <span className="min-w-0 flex-1 break-all">
                    {n.path}{' '}
                    <span className="text-neutral-500">
                      {n.task_display_id}, by {authorLabel(n)}, version recorded {ageLabel(n.written_at, nowMs)}
                    </span>
                  </span>
                  <span className={badge.className} title={badge.title}>
                    {badge.label}
                  </span>
                </li>
              )
            })}
          </ul>
        </div>
      )}

      <p className="text-[11px] text-neutral-500">
        As of {freshnessLines(view.freshness, nowMs).join(' · ')}. Generated now. Status and ownership come from the task
        system only; a note edit is not progress.
      </p>
    </div>
  )
}

/** The briefing panel above a selected initiative's tasks. */
export function InitiativeBriefing({ initiativeId, name, onClose }: { initiativeId: string; name: string; onClose: () => void }) {
  const found = usePolled(initiativeId, () => fetchInitiativeActivityClient(initiativeId), BRIEFING_POLL_MS)
  useRefreshOn(['task:sync'], found.refresh)
  return (
    <section aria-label={`Briefing for ${name}`} className="mb-4 rounded-2xl hq-card p-3">
      <div className="flex items-center justify-between mb-2">
        <h2 className="text-[11px] font-bold uppercase tracking-wider text-neutral-400">Briefing: {name}</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close briefing"
          className="p-1 rounded-lg text-neutral-400 hover:text-neutral-200 hover:bg-white/10"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
      {found.loading ? (
        <div role="status" className="flex items-center gap-2 text-xs text-neutral-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading briefing
        </div>
      ) : found.data ? (
        <InitiativeBriefingView view={found.data} />
      ) : (
        <p role="alert" className="text-xs text-rose-400">
          {found.error ?? 'Could not load the briefing.'}
        </p>
      )}
    </section>
  )
}
