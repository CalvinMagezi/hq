import type { Briefing } from './verificationApi'
import { t, tn } from '~/strings'
import { countdown } from '~/components/tasks/verification/verification'

export type AwaySectionId = 'finishing' | 'finished' | 'back' | 'waiting' | 'spot'

export interface AwayRow {
  key: string
  taskId?: string
  title: string
  detail: string
}

export interface AwaySection {
  id: AwaySectionId
  title: string
  count: number
  rows: AwayRow[]
}

/** The line in the Tasks list that opens the away page. Null when HQ finished nothing. */
export function awayLinkLine(b: Pick<Briefing, 'completed_total'> | null | undefined): string | null {
  return b && b.completed_total > 0 ? tn('tasks_away_link_line', b.completed_total) : null
}

/** The away page's sections in the phone's order: finishing soon, finished, went back, waiting on you, spot checks. */
export function awaySections(b: Briefing, nowMs: number, owner: boolean): AwaySection[] {
  const out: AwaySection[] = []
  const add = (id: AwaySectionId, title: string, rows: AwayRow[], count = rows.length) => {
    if (count > 0) out.push({ id, title, count, rows })
  }
  add(
    'finishing',
    t('tasks_away_finishing'),
    b.veto_windows.map((w) => ({
      key: w.task_id,
      taskId: w.task_id,
      title: `${w.display_id} ${w.title}`,
      detail: t('tasks_away_finishes_at', { time: countdown(w.release_at, nowMs) }),
    })),
  )
  add(
    'finished',
    t('tasks_away_finished'),
    b.completed.map((g) => ({
      key: g.initiative_id,
      title: tn('tasks_away_group', g.count, { area: g.initiative || t('check_other_area') }),
      detail: '',
    })),
    b.completed_total,
  )
  add(
    'back',
    t('tasks_away_back_you'),
    b.reworked.map((r) => ({
      key: r.task_id,
      taskId: r.task_id,
      title: `${r.display_id} ${r.title}`,
      detail: r.last_finding || t('tasks_away_failed'),
    })),
    b.reworked_total,
  )
  add(
    'waiting',
    t('tasks_away_waiting'),
    b.needs_you.map((i) => ({ key: i.task_id, taskId: i.task_id, title: `${i.display_id} ${i.title}`, detail: i.reason })),
    b.needs_you_total,
  )
  if (owner) add('spot', t('tasks_away_spot'), [], b.open_audits)
  return out
}
