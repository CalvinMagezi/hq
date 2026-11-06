import { Loader2, X } from 'lucide-react'
import { fetchInitiativeActivityClient, type InitiativeActivity } from '~/lib/taskActivityApi'
import { STATUS_LABELS, type TaskStatus } from '~/lib/tasksApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { ageLabel, authorLabel, briefingIsEmpty, freshnessLines, reviewBadge, reviewSummary } from './activity'
import { t } from '~/strings'
import './leaf.css'

const BRIEFING_POLL_MS = 120_000

const statusLabel = (status: string) => STATUS_LABELS[status as TaskStatus] ?? status

/** The briefing for one initiative. Presentational, like the task panel. */
export function InitiativeBriefingView({ view, nowMs }: { view: InitiativeActivity; nowMs?: number }) {
  if (briefingIsEmpty(view)) {
    return <p className="a-body-sm a-muted">{view.initiative.name} has no active tasks, so there is nothing to brief.</p>
  }
  const counts = Object.entries(view.status_counts)
  return (
    <div className="l-stack" data-scope="initiative">
      <p className="a-body-sm">
        {counts.map(([status, n]) => `${n} ${statusLabel(status).toLowerCase()}`).join(', ')}
        {view.tasks_shown > 0 ? ` (${view.tasks_shown} tasks)` : ''}
      </p>

      {view.awaiting_review.length > 0 && (
        <div className="l-stack">
          <div className="a-label">{t('brief_waiting_review')}</div>
          <ul className="a-body-sm">
            {view.awaiting_review.map((t) => (
              <li key={t.id}>
                {t.display_id} {t.title}
              </li>
            ))}
          </ul>
        </div>
      )}

      {view.blocked.length > 0 && (
        <div className="l-stack">
          <div className="a-label">{t('brief_blocked')}</div>
          <ul className="a-body-sm">
            {view.blocked.map((t) => (
              <li key={t.id} className="l-break">
                {t.display_id} {t.title}
                {t.reason ? <span className="a-muted">: {t.reason}</span> : null}
                {t.waiting_on ? <span className="a-muted"> (waiting on {t.waiting_on})</span> : null}
              </li>
            ))}
          </ul>
        </div>
      )}

      {view.contributors.length > 0 ? (
        <div className="l-stack">
          <div className="a-label">{t('brief_contributors')}</div>
          <ul className="a-body-sm">
            {view.contributors.map((c) => (
              <li key={c.actor}>
                {c.actor}{' '}
                <span className="a-muted">
                  {c.notes} {c.notes === 1 ? 'note' : 'notes'}
                  {c.leases > 0 ? `, ${c.leases} ${c.leases === 1 ? 'lease' : 'leases'}` : ''}
                  {c.last_active ? `, last ${ageLabel(c.last_active, nowMs)}` : ''}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p className="a-body-sm a-muted">{t('brief_no_contributors')}</p>
      )}

      <p className="a-body-sm">
        Notes: {reviewSummary(view.review_counts)}. {view.gaps_total} {view.gaps_total === 1 ? 'gap' : 'gaps'} in attribution or links.
      </p>

      {view.recent_notes.length > 0 && (
        <div className="l-stack">
          <div className="a-label">{t('brief_recent_notes')}</div>
          <ul className="l-stack">
            {view.recent_notes.map((n) => {
              const badge = reviewBadge(n)
              return (
                <li key={`${n.task_display_id}-${n.path}`} className="l-row a-body-sm">
                  <span className="l-grow l-break">
                    {n.path}{' '}
                    <span className="a-muted">
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

      <p className="a-cap a-muted">
        As of {freshnessLines(view.freshness, nowMs).join(' · ')}. Generated now. Status and ownership come from the task
        system only; a note edit is not progress.
      </p>
    </div>
  )
}

/** The briefing panel above a selected initiative's tasks. */
export function InitiativeBriefing({ initiativeId, name, onClose }: { initiativeId: string; name: string; onClose: () => void }) {
  const found = usePolled(initiativeId, () => fetchInitiativeActivityClient(initiativeId), BRIEFING_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  return (
    <section aria-label={`Briefing for ${name}`} className="a-card mb-4">
      <div className="l-row justify-between">
        <h2 className="a-label">Briefing: {name}</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label={t('brief_close')}
          className="a-ib sm"
        >
          <X className="a-ic sm" aria-hidden="true" />
        </button>
      </div>
      {found.loading ? (
        <div role="status" className="a-loading">
          <Loader2 className="a-ic xs animate-spin" aria-hidden="true" />
          {t('brief_loading')}
        </div>
      ) : found.data ? (
        <InitiativeBriefingView view={found.data} />
      ) : (
        <p role="alert" className="a-body-sm a-danger-t">
          {found.error ?? 'Could not load the briefing.'}
        </p>
      )}
    </section>
  )
}
