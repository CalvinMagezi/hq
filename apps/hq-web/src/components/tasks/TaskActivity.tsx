import { useEffect, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { AlertCircle, Loader2 } from 'lucide-react'
import { fetchTaskActivityClient, type ActivityGap, type ActivityNote, type TaskActivity as TaskActivityData } from '~/lib/taskActivityApi'
import type { TaskItem } from '~/lib/tasksApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { t, type StringKey } from '~/strings'
import './leaf.css'
import {
  ageLabel,
  authorLabel,
  gapLine,
  lastActivityLine,
  panelKind,
  reviewBadge,
  reviewSummary,
  unattributedCount,
  type PanelKind,
} from './activity'

const ACTIVITY_POLL_MS = 120_000
/** Gaps shown before the rest fold away. */
const GAPS_SHOWN = 4
const DECISION_STATES = ['accepted', 'reviewed', 'unreviewed', 'disputed', 'superseded'] as const

export type NoteRenderer = (path: string) => ReactNode

const plainPath: NoteRenderer = (path) => <span className="l-break">{path}</span>

const vaultLink: NoteRenderer = (path) => (
  <Link to="/notes/$" params={{ _splat: path }} className="a-link l-break">
    {path}
  </Link>
)

function ReviewBadge({ note }: { note: ActivityNote }) {
  const badge = reviewBadge(note)
  return (
    <span className={badge.className} title={badge.title}>
      {badge.label}
    </span>
  )
}

function NoteRow({ note, renderPath, nowMs }: { note: ActivityNote; renderPath: NoteRenderer; nowMs?: number }) {
  const reviewed = note.reviewed_by ? `${note.reviewed_by}${note.review_reason ? `: ${note.review_reason}` : ''}` : null
  return (
    <li className="a-drow l-tall l-col">
      <div className="l-row a-body-sm">
        <span className="l-grow">{renderPath(note.path)}</span>
        <ReviewBadge note={note} />
      </div>
      <p className="a-cap a-muted">
        By {authorLabel(note)}
        {note.session ? `, session ${note.session}` : ''}
        {note.written_at ? (
          <span title="When this version was recorded. It says nothing about how far the work has got.">
            {' '}
            · version recorded {ageLabel(note.written_at, nowMs)}
          </span>
        ) : null}
        {note.recorded_late ? ' · linked to its author later' : ''}
      </p>
      {reviewed && <p className="a-cap a-muted l-break">Decision by {reviewed}</p>}
    </li>
  )
}

function Gaps({ gaps, total }: { gaps: readonly ActivityGap[]; total: number }) {
  if (gaps.length === 0) return null
  const shown = gaps.slice(0, GAPS_SHOWN)
  const rest = gaps.slice(GAPS_SHOWN)
  const row = (g: ActivityGap, i: number) => (
    <li key={`${g.kind}-${g.subject ?? ''}-${i}`} className="a-why l-break">
      <AlertCircle className="a-ic xs" aria-hidden="true" /><span>{gapLine(g)}</span>
    </li>
  )
  return (
    <div className="l-pad">
      <div className="a-label mb-1">{t('act_worth_a_look', { n: total })}</div>
      <ul className="l-stack">{shown.map(row)}</ul>
      {rest.length > 0 && (
        <details className="l-details mt-1">
          <summary className="a-cap a-muted">{rest.length} more</summary>
          <ul className="l-stack mt-1">{rest.map((g, i) => row(g, i + GAPS_SHOWN))}</ul>
        </details>
      )}
    </div>
  )
}

const KIND_NOTE: Record<PanelKind, StringKey | null> = {
  contributors: null,
  unattributed_only: 'act_unattributed_only',
  none: 'act_none',
}

/** The activity view for one task. Presentational: the data is passed in, so it renders the same for any fixture. */
export function TaskActivityView({
  view,
  renderPath = plainPath,
  nowMs,
}: {
  view: TaskActivityData
  renderPath?: NoteRenderer
  nowMs?: number
}) {
  const kind = panelKind(view)
  const missingAuthors = unattributedCount(view.notes)
  const decisions = DECISION_STATES.filter((s) => (view.decisions_and_findings[s]?.length ?? 0) > 0)
  return (
    <section aria-label="Activity" data-kind={kind}>
      <SectionLabel>Activity</SectionLabel>
      <div className="a-card l-lines">
        {KIND_NOTE[kind] && <p className="a-body-sm a-muted l-pad">{t(KIND_NOTE[kind])}</p>}

        {view.holders.length > 0 && (
          <div className="l-pad">
            <div className="a-label">{t('act_held_by')}</div>
            <ul className="a-body-sm">
              {view.holders.map((h) => (
                <li key={h.lease_id}>
                  {h.actor}
                  {h.harness ? ` on ${h.harness}` : ''}, {t('act_holder_last', { ago: ageLabel(h.last_heartbeat_at, nowMs) })}
                </li>
              ))}
            </ul>
          </div>
        )}

        {view.sessions.length > 0 && (
          <div className="l-pad">
            <div className="a-label">Sessions</div>
            <ul className="a-body-sm">
              {view.sessions.map((s) => (
                <li key={s.id}>
                  {s.registered === false ? (
                    <span className="a-warn-t">{t('act_session_unknown', { id: s.id })}</span>
                  ) : (
                    <>
                      {s.agent || s.harness || s.id}{' '}
                      <span className="a-muted">
                        {s.running ? 'running' : (s.status ?? 'unknown')}
                        {s.last_seen_at ? `, seen ${ageLabel(s.last_seen_at, nowMs)}` : ''}
                      </span>
                    </>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        {view.contributors.length > 0 && (
          <div className="l-pad">
            <div className="a-label">Contributors</div>
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
        )}

        {view.notes.length > 0 && (
          <div className="l-pad">
            <div className="a-label mb-1">
              Notes ({view.notes_total}) · {reviewSummary(view.review_counts)}
              {missingAuthors > 0 ? ` · ${missingAuthors} with no known author` : ''}
            </div>
            <ul>
              {view.notes.map((n) => (
                <NoteRow key={n.path} note={n} renderPath={renderPath} nowMs={nowMs} />
              ))}
            </ul>
          </div>
        )}

        {decisions.length > 0 && (
          <div className="l-pad">
            <div className="a-label mb-1">Decisions and findings</div>
            {decisions.map((state) => (
              <div key={state}>
                <div className="a-body-sm-b capitalize">{state}</div>
                <ul className="a-body-sm">
                  {(view.decisions_and_findings[state] ?? []).map((n) => (
                    <li key={n.path}>
                      {renderPath(n.path)} <span className="a-muted">({n.type})</span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}

        {view.open_questions.length > 0 && (
          <div className="l-pad">
            <div className="a-label">Open questions (as recorded)</div>
            <ul className="a-body-sm">
              {view.open_questions.map((q) => (
                <li key={`${q.source}-${q.at}`} className="l-pre">
                  {q.text} <span className="a-muted">({q.recorded_by}, {ageLabel(q.at, nowMs)})</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <Gaps gaps={view.gaps} total={view.gaps_total} />

        <p className="a-cap a-muted l-pad">
          {lastActivityLine(view.freshness, nowMs)}
        </p>
      </div>
    </section>
  )
}

/** The activity panel in the task drawer: fetches the view and keeps it current. */
export function TaskActivity({ task }: { task: TaskItem }) {
  const found = usePolled(task.id, () => fetchTaskActivityClient(task.id), ACTIVITY_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  const { refresh } = found
  useEffect(() => {
    void refresh()
  }, [task.updated_at, refresh])

  if (found.loading) {
    return (
      <section aria-label="Activity">
        <SectionLabel>Activity</SectionLabel>
        <div role="status" className="a-loading">
          <Loader2 className="a-ic xs animate-spin" aria-hidden="true" />
          Loading activity
        </div>
      </section>
    )
  }
  if (!found.data) {
    return (
      <section aria-label="Activity">
        <SectionLabel>Activity</SectionLabel>
        <p role="alert" className="a-body-sm a-danger-t">
          {found.error ?? 'Could not load activity.'}
        </p>
      </section>
    )
  }
  return <TaskActivityView view={found.data} renderPath={vaultLink} />
}
