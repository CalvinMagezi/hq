import { useEffect, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { fetchTaskActivityClient, type ActivityGap, type ActivityNote, type TaskActivity as TaskActivityData } from '~/lib/taskActivityApi'
import type { TaskItem } from '~/lib/tasksApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import {
  ageLabel,
  authorLabel,
  freshnessLines,
  gapTitle,
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

const plainPath: NoteRenderer = (path) => <span className="break-all">{path}</span>

const vaultLink: NoteRenderer = (path) => (
  <Link to="/vault/$" params={{ _splat: path }} className="break-all text-neutral-200 hover:underline">
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
    <li className="px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5 space-y-0.5">
      <div className="flex items-start gap-2 text-xs">
        <span className="min-w-0 flex-1">{renderPath(note.path)}</span>
        <ReviewBadge note={note} />
      </div>
      <p className="text-[11px] text-neutral-500">
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
      {reviewed && <p className="text-[11px] text-neutral-400 break-words">Decision by {reviewed}</p>}
    </li>
  )
}

function Gaps({ gaps, total }: { gaps: readonly ActivityGap[]; total: number }) {
  if (gaps.length === 0) return null
  const shown = gaps.slice(0, GAPS_SHOWN)
  const rest = gaps.slice(GAPS_SHOWN)
  const row = (g: ActivityGap, i: number) => (
    <li key={`${g.kind}-${g.subject ?? ''}-${i}`} className="text-[11px] text-amber-300/90 break-words">
      <span className="font-semibold">{gapTitle(g)}</span>
      {g.subject ? `: ${g.subject}` : ''}. {g.detail}.
    </li>
  )
  return (
    <div>
      <div className="text-[11px] uppercase tracking-wider text-neutral-500 mb-1">Gaps ({total})</div>
      <ul className="space-y-1">{shown.map(row)}</ul>
      {rest.length > 0 && (
        <details className="mt-1">
          <summary className="text-[11px] text-neutral-500 cursor-pointer">{rest.length} more</summary>
          <ul className="space-y-1 mt-1">{rest.map((g, i) => row(g, i + GAPS_SHOWN))}</ul>
        </details>
      )}
    </div>
  )
}

const KIND_NOTE: Record<PanelKind, string | null> = {
  contributors: null,
  unattributed_only: 'Changes are linked to this task but none has a known author. HQ does not guess one.',
  none: 'Nothing has been recorded on this task: no lease, session or note. Its status is whatever the task says.',
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
      <div className="space-y-3 px-3 py-2.5 rounded-xl bg-white/[0.02] border border-white/5">
        {KIND_NOTE[kind] && <p className="text-xs text-neutral-400">{KIND_NOTE[kind]}</p>}

        {view.holders.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500">Held by</div>
            <ul className="text-xs text-neutral-200">
              {view.holders.map((h) => (
                <li key={h.lease_id}>
                  {h.actor}
                  {h.harness ? ` on ${h.harness}` : ''}, last heartbeat {ageLabel(h.last_heartbeat_at, nowMs)}
                </li>
              ))}
            </ul>
          </div>
        )}

        {view.sessions.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500">Sessions</div>
            <ul className="text-xs text-neutral-200 space-y-0.5">
              {view.sessions.map((s) => (
                <li key={s.id}>
                  {s.registered === false ? (
                    <span className="text-amber-300/90">{s.id}: not in the session registry</span>
                  ) : (
                    <>
                      {s.agent || s.harness || s.id}{' '}
                      <span className="text-neutral-500">
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
        )}

        {view.notes.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500 mb-1">
              Notes ({view.notes_total}) · {reviewSummary(view.review_counts)}
              {missingAuthors > 0 ? ` · ${missingAuthors} with no known author` : ''}
            </div>
            <ul className="space-y-1.5">
              {view.notes.map((n) => (
                <NoteRow key={n.path} note={n} renderPath={renderPath} nowMs={nowMs} />
              ))}
            </ul>
          </div>
        )}

        {decisions.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500 mb-1">Decisions and findings</div>
            {decisions.map((state) => (
              <div key={state} className="mb-1.5">
                <div className="text-[11px] font-semibold text-neutral-300 capitalize">{state}</div>
                <ul className="text-xs text-neutral-300 space-y-0.5">
                  {(view.decisions_and_findings[state] ?? []).map((n) => (
                    <li key={n.path}>
                      {renderPath(n.path)} <span className="text-neutral-500">({n.type})</span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}

        {view.open_questions.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500">Open questions (as recorded)</div>
            <ul className="text-xs text-neutral-200 space-y-0.5">
              {view.open_questions.map((q) => (
                <li key={`${q.source}-${q.at}`} className="whitespace-pre-wrap break-words">
                  {q.text} <span className="text-neutral-500">({q.recorded_by}, {ageLabel(q.at, nowMs)})</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <Gaps gaps={view.gaps} total={view.gaps_total} />

        <p className="text-[11px] text-neutral-500">
          As of {freshnessLines(view.freshness, nowMs).join(' · ')}. Generated now from the task system, sessions and
          note records. A note being edited is not progress, and status comes only from the task.
        </p>
      </div>
    </section>
  )
}

/** The activity panel in the task drawer: fetches the view and keeps it current. */
export function TaskActivity({ task }: { task: TaskItem }) {
  const found = usePolled(task.id, () => fetchTaskActivityClient(task.id), ACTIVITY_POLL_MS)
  useRefreshOn(['task:sync'], found.refresh)
  const { refresh } = found
  useEffect(() => {
    void refresh()
  }, [task.updated_at, refresh])

  if (found.loading) {
    return (
      <section aria-label="Activity">
        <SectionLabel>Activity</SectionLabel>
        <div role="status" className="flex items-center gap-2 text-xs text-neutral-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading activity
        </div>
      </section>
    )
  }
  if (!found.data) {
    return (
      <section aria-label="Activity">
        <SectionLabel>Activity</SectionLabel>
        <p role="alert" className="text-xs text-rose-400">
          {found.error ?? 'Could not load activity.'}
        </p>
      </section>
    )
  }
  return <TaskActivityView view={found.data} renderPath={vaultLink} />
}
