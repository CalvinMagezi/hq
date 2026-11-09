// HQ's own work: the runs the edge worker took on, shown on the Helpers page as read-only cards.

import { hqJson } from '~/lib/hqAuth'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { t } from '~/strings'

export interface EdgeRun {
  ask_id: string
  thread_id: string | null
  status: 'pending' | 'answered' | 'failed'
  started_at: string
  finished_at: string | null
  task_id: string | null
  task: { display_id: string; title: string; status: string } | null
}

export interface EdgeWorkerReport {
  enabled: boolean
  tag?: string
  repos?: string[]
  runs: EdgeRun[]
  task_budget_usd?: number
}

export const edgeWorkerApi = {
  runs: async (): Promise<EdgeRun[]> => (await hqJson<EdgeWorkerReport>('/api/edge-worker')).runs ?? [],
  taskBudget: async (): Promise<number> => (await hqJson<EdgeWorkerReport>('/api/edge-worker')).task_budget_usd ?? 0,
}

export type OwnState = 'working' | 'finished' | 'stopped'

/** Pending is working, answered is finished, anything else counts as stopped (as on the phone). */
export const ownState = (run: Pick<EdgeRun, 'status'>): OwnState =>
  run.status === 'pending' ? 'working' : run.status === 'answered' ? 'finished' : 'stopped'

/** The id a card for this run carries, never a helper id. */
export const ownCardId = (run: Pick<EdgeRun, 'ask_id'>) => `edge:${run.ask_id}`

export const ownTitle = (run: Pick<EdgeRun, 'task' | 'ask_id'>) => run.task?.title ?? run.task?.display_id ?? run.ask_id

export const ownWords = (run: Pick<EdgeRun, 'status'>): string =>
  ownState(run) === 'working' ? t('helpers_own_working') : ownState(run) === 'finished' ? t('helpers_own_done') : t('helper_state_stopped')

export const ownSince = (run: Pick<EdgeRun, 'started_at'>): number => parseSqliteUtc(run.started_at).getTime()
