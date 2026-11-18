import { create } from 'zustand'
import { CLOSED_PANE, type PaneState, type PaneTab } from '~/lib/workPane'

interface WorkPaneStore {
  byThread: Record<string, PaneState>
  /** The state of one chat's pane; a chat never seen is closed. */
  get: (threadId: string | null | undefined) => PaneState
  toggle: (threadId: string) => void
  close: (threadId: string) => void
  showTab: (threadId: string, tab: PaneTab) => void
  /** Opens the Result tab on a note. */
  showNote: (threadId: string, path: string) => void
  /** Opens the Context tab on a source. */
  showSource: (threadId: string, ref: string | null) => void
}

export const useWorkPane = create<WorkPaneStore>((set, get) => {
  const patch = (threadId: string, change: Partial<PaneState>) =>
    set((s) => ({ byThread: { ...s.byThread, [threadId]: { ...(s.byThread[threadId] ?? CLOSED_PANE), ...change } } }))
  return {
    byThread: {},
    get: (threadId) => (threadId ? get().byThread[threadId] ?? CLOSED_PANE : CLOSED_PANE),
    toggle: (threadId) => patch(threadId, { open: !(get().byThread[threadId]?.open ?? false) }),
    close: (threadId) => patch(threadId, { open: false }),
    showTab: (threadId, tab) => patch(threadId, { open: true, tab }),
    showNote: (threadId, path) => patch(threadId, { open: true, tab: 'result', note: path }),
    showSource: (threadId, ref) => patch(threadId, { open: true, tab: 'context', source: ref }),
  }
})

/** One chat's pane state, re-rendering only when it changes. */
export function usePane(threadId: string | null | undefined): PaneState {
  return useWorkPane((s) => (threadId ? s.byThread[threadId] ?? CLOSED_PANE : CLOSED_PANE))
}
