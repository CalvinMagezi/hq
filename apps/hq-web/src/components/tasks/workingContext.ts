import { createContext, useContext } from 'react'
import type { WorkSession } from '~/lib/tasksApi'

/** Task id to the name of whoever holds a live work lease on it, so any card can say so without props. */
export const WorkingNowContext = createContext<ReadonlyMap<string, string>>(new Map())

export function workingNow(sessions: readonly WorkSession[]): ReadonlyMap<string, string> {
  const live = new Map<string, string>()
  for (const s of sessions) if (s.ended_at === null) live.set(s.task_id, s.actor)
  return live
}

export function useWorkingNow(taskId: string): string | undefined {
  return useContext(WorkingNowContext).get(taskId)
}
