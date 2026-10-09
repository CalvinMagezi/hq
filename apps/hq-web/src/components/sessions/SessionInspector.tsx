import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { agentName, computerName, statusInfo } from '~/lib/workbench'
import { SessionTaskLink } from './SessionRow'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 text-sm">
      <dt className="text-neutral-400 shrink-0">{label}</dt>
      <dd className="min-w-0 text-right text-neutral-100 break-words">{children}</dd>
    </div>
  )
}

/** Facts about the open agent, shown beside it on large screens where there is room to keep them in view. */
export function SessionInspector({ session: s }: { session: HarnessSession }) {
  const status = statusInfo(s)
  const started = ago(s.created_at)
  const seen = ago(s.last_seen_at)
  return (
    <aside aria-label="Agent details" className="flex flex-col gap-4 w-[340px] shrink-0 overflow-y-auto p-5 border-l border-white/10">
      <h2 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">Details</h2>
      {status.agent === 'blocked' && (
        <div className="hq-card rounded-xl p-4" style={{ borderColor: 'var(--accent-blue)' }}>
          <p className="text-sm font-semibold text-neutral-50">Waiting for you</p>
          <p className="mt-1 text-xs text-neutral-400">Nothing is answered on your behalf. Use the terminal to choose.</p>
        </div>
      )}
      <dl className="hq-card rounded-xl p-4 flex flex-col gap-3">
        <Row label="Status">{status.word}</Row>
        <Row label="Runtime">{agentName(s.harness)}</Row>
        <Row label="Folder">{s.cwd || 'Unknown'}</Row>
        <Row label="Computer">{computerName(s.host)}</Row>
        {started && <Row label="Started">{started} ago</Row>}
        {seen && <Row label="Last seen">{seen} ago</Row>}
        <Row label="HQ">{s.mode === 'drive' && s.drive ? 'Driving' : 'Watching'}</Row>
      </dl>
      {s.goal && (
        <section className="hq-card rounded-xl p-4">
          <h3 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400 mb-2">Goal</h3>
          <p className="text-sm text-neutral-100 leading-relaxed">{s.goal}</p>
          {s.done_criteria && <p className="mt-2 text-xs text-neutral-400 leading-relaxed">Done when: {s.done_criteria}</p>}
        </section>
      )}
      {s.task && (
        <section className="hq-card rounded-xl p-4">
          <h3 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400 mb-2">Linked task</h3>
          <SessionTaskLink session={s} />
        </section>
      )}
    </aside>
  )
}
