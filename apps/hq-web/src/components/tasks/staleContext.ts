import { createContext, useContext } from 'react'

/** Ids of in-progress tasks nobody holds or has touched lately, so any card can say so without props. */
export const StaleIdsContext = createContext<ReadonlySet<string>>(new Set())

/** The previous set when nothing changed, so a poll that finds the same tasks does not re-render every card. */
export function stableSet(next: ReadonlySet<string>, previous: ReadonlySet<string>): ReadonlySet<string> {
  if (next.size !== previous.size) return next
  for (const id of next) if (!previous.has(id)) return next
  return previous
}

export function useIsStale(taskId: string): boolean {
  return useContext(StaleIdsContext).has(taskId)
}
