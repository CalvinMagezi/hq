import { t } from '~/strings'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { agentName, computerName, statusInfo } from '~/lib/workbench'
import { HelperPill, SessionTaskLink } from './SessionRow'

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
    <aside aria-label={t('helpers_details_aria')} className="flex flex-col gap-4 w-[340px] shrink-0 overflow-y-auto p-5 border-l border-white/10">
      <h2 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400">{t('inbox_details')}</h2>
      {status.agent === 'blocked' && (
        <div className="hq-card hq-card-needs rounded-xl p-4">
          <p className="text-sm font-semibold text-neutral-50">{t('helpers_waiting_for_you')}</p>
          <p className="mt-1 text-xs text-neutral-400">{t('helpers_answer_in_terminal')}</p>
        </div>
      )}
      <dl className="hq-card rounded-xl p-4 flex flex-col gap-3">
        <Row label="Status"><HelperPill session={s} /></Row>
        <Row label="Runtime">{agentName(s.harness)}</Row>
        <Row label="Folder">{s.cwd || 'Unknown'}</Row>
        <Row label="Computer">{computerName(s.host)}</Row>
        {started && <div className="text-sm text-neutral-100">{t('helper_started', { when: started })}</div>}
        {seen && <div className="text-sm text-neutral-100">{t('helper_last_seen', { when: seen })}</div>}
        <Row label="HQ">{s.mode === 'drive' && s.drive ? 'Driving' : 'Watching'}</Row>
      </dl>
      {s.goal && (
        <section className="hq-card rounded-xl p-4">
          <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400 mb-2">Goal</h3>
          <p className="text-sm text-neutral-100 leading-relaxed">{s.goal}</p>
          {s.done_criteria && <p className="mt-2 text-xs text-neutral-400 leading-relaxed">Done when: {s.done_criteria}</p>}
        </section>
      )}
      {s.task && (
        <section className="hq-card rounded-xl p-4">
          <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400 mb-2">Linked task</h3>
          <SessionTaskLink session={s} />
        </section>
      )}
    </aside>
  )
}
