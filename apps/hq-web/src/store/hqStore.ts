import { create } from 'zustand'
import { createJSONStorage, persist } from 'zustand/middleware'
import { idbStateStorage } from '~/lib/offlineCache'

import { compareTasksByActivity } from '~/components/tasks/hierarchy'

interface HQState {
  wsConnected: boolean
  setWsConnected: (v: boolean) => void

  globalChatOpen: boolean
  setGlobalChatOpen: (v: boolean) => void
  chatUnreadCount: number
  bumpChatUnread: () => void
  clearChatUnread: () => void

  /** Text a note action hands the chat overlay to prefill its input. */
  chatDraft: string | null
  setChatDraft: (text: string | null) => void

  notifications: import('~/lib/notificationsApi').NotificationItem[]
  setNotifications: (items: import('~/lib/notificationsApi').NotificationItem[]) => void
  unreadNotificationsCount: number
  pendingApprovalsCount: number
  setNotificationCounts: (unread: number, pending: number) => void
  /** Coding agents waiting on the person; drives the Workbench nav badge. Not persisted. */
  needsYouCount: number
  setNeedsYouCount: (count: number) => void
  updateNotificationState: (id: string, state: import('~/lib/notificationsApi').NotificationState) => void

  systemNotice: string | null
  setSystemNotice: (message: string | null) => void

  tasks: import('~/lib/tasksApi').TaskItem[]
  setTasks: (tasks: import('~/lib/tasksApi').TaskItem[]) => void
  upsertTask: (task: import('~/lib/tasksApi').TaskItem) => void
  removeTask: (id: string) => void
  selectedTaskId: string | null
  setSelectedTaskId: (id: string | null) => void
  spaces: import('~/lib/tasksApi').Space[]
  setSpaces: (spaces: import('~/lib/tasksApi').Space[]) => void
  folders: import('~/lib/tasksApi').Folder[]
  setFolders: (folders: import('~/lib/tasksApi').Folder[]) => void
  initiatives: import('~/lib/tasksApi').Initiative[]
  setInitiatives: (initiatives: import('~/lib/tasksApi').Initiative[]) => void
}

export const useHQStore = create<HQState>()(
  persist(
    (set) => ({
      wsConnected: false,
      setWsConnected: (wsConnected) => set({ wsConnected }),

      globalChatOpen: false,
      setGlobalChatOpen: (v) => set({ globalChatOpen: v }),
      chatUnreadCount: 0,
      bumpChatUnread: () => set((s) => ({ chatUnreadCount: s.chatUnreadCount + 1 })),
      clearChatUnread: () => set({ chatUnreadCount: 0 }),

      chatDraft: null,
      setChatDraft: (chatDraft) => set({ chatDraft }),

      notifications: [],
      setNotifications: (notifications) => set({ notifications }),
      unreadNotificationsCount: 0,
      pendingApprovalsCount: 0,
      setNotificationCounts: (unreadNotificationsCount, pendingApprovalsCount) =>
        set({ unreadNotificationsCount, pendingApprovalsCount }),
      needsYouCount: 0,
      setNeedsYouCount: (needsYouCount) => set({ needsYouCount }),
      updateNotificationState: (id, state) =>
        set((s) => {
          const next = s.notifications.map((n) => (n.id === id ? { ...n, state } : n))
          const unread = next.filter((n) => n.state === 'pending').length
          const pending = next.filter(
            (n) => n.state === 'pending' && n.kind === 'action_needed'
          ).length
          return { notifications: next, unreadNotificationsCount: unread, pendingApprovalsCount: pending }
        }),

      systemNotice: null,
      setSystemNotice: (systemNotice) => set({ systemNotice }),

      tasks: [],
      setTasks: (tasks) => set({ tasks: [...tasks].sort(compareTasksByActivity) }),
      upsertTask: (task) =>
        set((s) => {
          const exists = s.tasks.some((t) => t.id === task.id)
          const tasks = exists ? s.tasks.map((t) => (t.id === task.id ? task : t)) : [task, ...s.tasks]
          return { tasks: tasks.sort(compareTasksByActivity) }
        }),
      removeTask: (id) => set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) })),
      selectedTaskId: null,
      setSelectedTaskId: (id) => set({ selectedTaskId: id }),
      spaces: [],
      setSpaces: (spaces) => set({ spaces }),
      folders: [],
      setFolders: (folders) => set({ folders }),
      initiatives: [],
      setInitiatives: (initiatives) => set({ initiatives }),
    }), {
    name: 'hq-store',
    // Lists that paint instantly on the next visit, refreshed from the server behind them.
    storage: createJSONStorage(() => idbStateStorage),
    merge: (persistedState, currentState) => {
      const persisted = (persistedState as Partial<HQState>) || {}
      return {
        ...currentState,
        ...persisted,
        tasks: Array.isArray(persisted.tasks)
          ? [...persisted.tasks].sort(compareTasksByActivity)
          : currentState.tasks,
      }
    },
    partialize: (s) => ({
      tasks: s.tasks,
      spaces: s.spaces,
      folders: s.folders,
      initiatives: s.initiatives,
      notifications: s.notifications,
      unreadNotificationsCount: s.unreadNotificationsCount,
      pendingApprovalsCount: s.pendingApprovalsCount,
    }),
  }))
