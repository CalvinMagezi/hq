import { useEffect } from 'react'
import { create } from 'zustand'
import { fetchThreadHelperRuns, type HelperRun, type HelperRunState } from '~/lib/helperRunsApi'

interface HelperRunsState {
  runsByThread: Record<string, HelperRun[]>
  loadedByThread: Record<string, boolean>
  unavailableByThread: Record<string, boolean>

  setThreadRuns: (threadId: string, runs: HelperRun[]) => void
  markUnavailable: (threadId: string) => void
  upsertRun: (threadId: string, run: HelperRun) => void
  upsertRuns: (threadId: string, runs: HelperRun[]) => void
  settleRun: (threadId: string, runId: string, state: HelperRunState) => void
  loadThreadRuns: (threadId: string) => Promise<void>
}

function mergeRunList(existing: HelperRun[], incoming: HelperRun[]): HelperRun[] {
  if (incoming.length === 0) return existing
  const byId = new Map(existing.map((r) => [r.id, r]))
  for (const run of incoming) byId.set(run.id, run)
  return [...byId.values()].sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0))
}

export const useHelperRunsStore = create<HelperRunsState>((set) => ({
  runsByThread: {},
  loadedByThread: {},
  unavailableByThread: {},

  setThreadRuns: (threadId, runs) =>
    set((s) => ({
      runsByThread: { ...s.runsByThread, [threadId]: runs },
      loadedByThread: { ...s.loadedByThread, [threadId]: true },
      unavailableByThread: { ...s.unavailableByThread, [threadId]: false },
    })),

  markUnavailable: (threadId) =>
    set((s) => ({
      unavailableByThread: { ...s.unavailableByThread, [threadId]: true },
    })),

  upsertRun: (threadId, run) =>
    set((s) => ({
      runsByThread: {
        ...s.runsByThread,
        [threadId]: mergeRunList(s.runsByThread[threadId] ?? [], [run]),
      },
      unavailableByThread: { ...s.unavailableByThread, [threadId]: false },
    })),

  upsertRuns: (threadId, runs) =>
    set((s) => {
      if (runs.length === 0) return s
      return {
        runsByThread: {
          ...s.runsByThread,
          [threadId]: mergeRunList(s.runsByThread[threadId] ?? [], runs),
        },
        unavailableByThread: { ...s.unavailableByThread, [threadId]: false },
      }
    }),

  settleRun: (threadId, runId, state) =>
    set((s) => {
      const list = s.runsByThread[threadId]
      if (!list) return s
      const now = Math.floor(Date.now() / 1000)
      return {
        runsByThread: {
          ...s.runsByThread,
          [threadId]: list.map((r) => (r.id === runId ? { ...r, state, settled_at: r.settled_at ?? now } : r)),
        },
      }
    }),

  loadThreadRuns: async (threadId) => {
    try {
      const runs = await fetchThreadHelperRuns(threadId)
      useHelperRunsStore.getState().setThreadRuns(threadId, runs)
    } catch {
      useHelperRunsStore.getState().markUnavailable(threadId)
    }
  },
}))

const EMPTY_RUNS: HelperRun[] = []

/** Loads and subscribes to a thread's helper runs; updates live via socket events. */
export function useThreadHelperRuns(threadId: string | null | undefined, connected = true) {
  const runs = useHelperRunsStore((s) => (threadId ? s.runsByThread[threadId] : undefined)) ?? EMPTY_RUNS
  const loaded = useHelperRunsStore((s) => (threadId ? s.loadedByThread[threadId] ?? false : false))
  const unavailable = useHelperRunsStore((s) => (threadId ? s.unavailableByThread[threadId] ?? false : false))

  useEffect(() => {
    if (!threadId || !connected) return
    void useHelperRunsStore.getState().loadThreadRuns(threadId)
  }, [threadId, connected])

  return { runs, loaded, unavailable }
}
