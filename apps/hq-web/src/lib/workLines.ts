// The state line under the Tasks and Helpers rows in Settings: whether anything waits, at a glance.
import { t } from '~/strings'
import type { TaskItem } from './tasksApi'
import type { HarnessSession } from './sessionsApi'
import { needsYou } from './workbench'

/** "12 open · 2 waiting on you". A task is waiting on the person when it is ready to check or stuck. */
export function tasksLine(tasks: Pick<TaskItem, 'status' | 'archived_at'>[]): string {
  const live = tasks.filter((x) => !x.archived_at && x.status !== 'complete')
  if (live.length === 0) return t('work_tasks_none')
  const waiting = live.filter((x) => x.status === 'ready_for_review' || x.status === 'blocked').length
  return waiting > 0 ? t('work_tasks_line', { open: String(live.length), waiting: String(waiting) }) : t('work_tasks_line_clear', { open: String(live.length) })
}

/** "3 working here · none need you". */
export function helpersLine(sessions: HarnessSession[]): string {
  const running = sessions.filter((s) => !s.archived && s.status === 'running')
  if (running.length === 0) return t('work_helpers_none')
  const needs = running.filter(needsYou).length
  return needs > 0 ? t('work_helpers_line', { working: String(running.length), needs: String(needs) }) : t('work_helpers_line_clear', { working: String(running.length) })
}
