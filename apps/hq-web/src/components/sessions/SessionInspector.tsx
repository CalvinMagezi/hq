import { t } from '~/strings'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { agentName, computerName } from '~/lib/workbench'
import { SessionTaskLink } from './SessionRow'
import { ActionError, FollowInChat, RenameTitle, StopButton, useAction } from './SessionActions'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 text-sm">
      <dt className="text-neutral-400 shrink-0">{label}</dt>
      <dd className="min-w-0 text-right text-neutral-100 break-words">{children}</dd>
    </div>
  )
}

/** Facts about the open helper, with Rename, Follow in Chat and Stop, shown beside it where there is room. */
export function SessionInspector({ session: s, onChanged }: { session: HarnessSession; onChanged?: () => void | Promise<void> }) {
  const action = useAction(onChanged ?? (() => undefined))
  const started = ago(s.created_at)
  const seen = ago(s.last_seen_at)
  const running = s.status === 'running'
  return (
    <aside aria-label={t('helpers_details_aria')} className="flex flex-col gap-4 w-[320px] shrink-0 overflow-y-auto p-5 border-l border-white/10">
      <h2 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400">{t('inbox_details')}</h2>
      <dl className="hq-card rounded-xl p-4 flex flex-col gap-3">
        <Row label={t('helpers_ins_helper')}>{agentName(s.harness)}</Row>
        <Row label={t('helpers_ins_computer')}>{computerName(s.host)}</Row>
        <Row label={t('helpers_ins_folder')}>{s.cwd || '-'}</Row>
        {started && <div className="text-sm text-neutral-100">{t('helper_started', { when: started })}</div>}
        {seen && <div className="text-sm text-neutral-100">{t('helper_last_seen', { when: seen })}</div>}
        <Row label={t('helpers_ins_task')}>{s.task ? <SessionTaskLink session={s} /> : '-'}</Row>
        <Row label={t('helpers_ins_chat')}>{s.owner_thread ? t('helpers_follow_open') : t('helpers_ins_chat_none')}</Row>
      </dl>
      {s.goal && (
        <section className="hq-card rounded-xl p-4">
          <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-neutral-400 mb-2">{t('helpers_ins_goal')}</h3>
          <p className="text-sm text-neutral-100 leading-relaxed">{s.goal}</p>
          {s.done_criteria && <p className="mt-2 text-xs text-neutral-400 leading-relaxed">{t('helpers_ins_done_when', { criteria: s.done_criteria })}</p>}
        </section>
      )}
      <div className="flex flex-col items-stretch gap-2">
        <RenameTitle session={s} action={action} />
        <FollowInChat session={s} action={action} />
        {running && <StopButton session={s} action={action} />}
        <ActionError error={action.error} />
      </div>
    </aside>
  )
}
