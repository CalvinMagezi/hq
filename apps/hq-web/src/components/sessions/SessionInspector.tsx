import { t } from '~/strings'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import type { HarnessSession } from '~/lib/sessionsApi'
import { agentName, computerName, folderName } from '~/lib/workbench'
import { SessionTaskLink } from './SessionRow'
import { ActionError, FollowInChat, RenameTitle, StopButton, useAction } from './SessionActions'

const ago = (when: string | null) => (when ? relTime(parseSqliteUtc(when)) : '')

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="a-drow">
      <span>{label}</span>
      <span>{children}</span>
    </div>
  )
}

/** Facts about the open helper, then Rename, Follow in Chat and Stop. The pane around it comes from Panes, which docks it from 1400 px and opens it as a sheet below. */
export function SessionInspector({ session: s, onChanged }: { session: HarnessSession; onChanged?: () => void | Promise<void> }) {
  const action = useAction(onChanged ?? (() => undefined))
  const started = ago(s.created_at)
  const seen = ago(s.last_seen_at)
  const running = s.status === 'running'
  return (
    <div className="flex flex-col overflow-y-auto">
      <div className="py-1">
        <Row label={t('helpers_ins_helper')}>{agentName(s.harness)}</Row>
        <Row label={t('helpers_ins_computer')}>{computerName(s.host)}</Row>
        <Row label={t('helpers_ins_folder')}>{folderName(s.cwd) || '-'}</Row>
        <Row label={t('helper_started_label')}>{started || '-'}</Row>
        <Row label={t('helpers_last_seen')}>{seen || '-'}</Row>
        <Row label={t('helpers_ins_task')}>{s.task ? <SessionTaskLink session={s} /> : '-'}</Row>
        <Row label={t('helpers_ins_chat')}>{s.owner_thread ? t('helpers_follow_open') : t('helpers_ins_chat_none')}</Row>
      </div>
      {s.goal && (
        <section className="flex flex-col gap-2 px-4 py-3">
          <h3 className="a-label">{t('helpers_ins_goal')}</h3>
          <p className="a-body-sm">{s.goal}</p>
          {s.done_criteria && <p className="a-body-sm a-muted">{t('helpers_ins_done_when', { criteria: s.done_criteria })}</p>}
        </section>
      )}
      <div className="flex flex-col items-stretch gap-2 p-4">
        <RenameTitle session={s} action={action} />
        <FollowInChat session={s} action={action} />
        {running && <StopButton session={s} action={action} />}
        <ActionError error={action.error} />
      </div>
    </div>
  )
}
