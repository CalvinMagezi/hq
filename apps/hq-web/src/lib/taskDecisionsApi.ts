import { hqJson } from './hqAuth'

/** One dialog a task's agents wait on the owner for (`GET /api/tasks/{id}/decisions`). */
export interface TaskDecision {
  item_id: string
  asked_at: string
  session_id: string
  seq: number
  kind: string
  question: string | null
  command: string | null
  options: { id: number; label: string; remember?: boolean; typed?: boolean }[]
  /** The one-time yes a button may pick; null when only the agent's own page can answer. */
  approve: { id: number; label: string | null } | null
}

export function fetchTaskDecisions(taskId: string): Promise<{ decisions: TaskDecision[] }> {
  return hqJson(`/api/tasks/${encodeURIComponent(taskId)}/decisions`)
}

/** Answers the dialog behind a notification item, as its notification button would. */
export function answerDecision(itemId: string, choice: 'approve' | 'decline'): Promise<unknown> {
  return hqJson(`/api/notifications/${encodeURIComponent(itemId)}/answer`, 'POST', { choice })
}
