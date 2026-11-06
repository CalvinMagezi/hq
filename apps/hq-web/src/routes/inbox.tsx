import { answerErrorLine } from '~/lib/inboxCard'
import { createFileRoute, useNavigate, useParams, useRouter } from '@tanstack/react-router'
import { useState, useEffect, useMemo, useCallback } from 'react'
import { useHQStore } from '~/store/hqStore'
import {
  fetchNotificationsClient,
  executeNotificationAction,
  markAllNotificationsRead,
  restoreNotification,
  restoreNotifications,
  clearableUpdateIds,
  inInboxTab,
  type InboxTab,
  type NotificationState,
} from '~/lib/notificationsApi'
import { toastUndo, toastAlert } from '~/lib/toast'
import { t, tn } from '~/strings'
import { failureLine } from '~/lib/version'
import { InboxView } from '~/components/notifications/InboxView'
import type { UpdateFilter } from '~/components/notifications/updateKind'

export const Route = createFileRoute('/inbox')({
  validateSearch: (search: Record<string, unknown>): { tab?: 'updates' | 'done' } => {
    const tab = search.tab === 'updates' || search.tab === 'done' ? search.tab : undefined
    return tab ? { tab } : {}
  },
  component: NotificationsPage,
})

const TAB_OF_SEARCH: Record<'updates' | 'done', InboxTab> = { updates: 'all', done: 'resolved' }

function NotificationsPage() {
  const { tab } = Route.useSearch()
  const linkedId = useParams({ strict: false }).id
  const navigate = useNavigate()
  const router = useRouter()
  const [activeTab, setActiveTab] = useState<InboxTab>(tab ? TAB_OF_SEARCH[tab] : 'pending')
  const [filter, setFilter] = useState<UpdateFilter>('all')
  const [searchQuery, setSearchQuery] = useState('')
  const [loadingAction, setLoadingAction] = useState<string | null>(null)
  const [isRefreshing, setIsRefreshing] = useState(false)
  const [checkedAt, setCheckedAt] = useState<Date | null>(null)

  const notifications = useHQStore((s) => s.notifications)
  const setNotifications = useHQStore((s) => s.setNotifications)
  const unreadCount = useHQStore((s) => s.unreadNotificationsCount)
  const pendingApprovalsCount = useHQStore((s) => s.pendingApprovalsCount)
  const setNotificationCounts = useHQStore((s) => s.setNotificationCounts)
  const updateNotificationState = useHQStore((s) => s.updateNotificationState)

  const loadData = useCallback(async () => {
    setIsRefreshing(true)
    try {
      const res = await fetchNotificationsClient({
        state: activeTab === 'resolved' ? 'resolved' : 'pending',
      })
      setNotifications(res.notifications)
      setNotificationCounts(res.unread_count, res.pending_approvals_count)
      setCheckedAt(new Date())
    } catch (e) {
      console.error('Failed to load notifications:', e)
    } finally {
      setIsRefreshing(false)
    }
  }, [activeTab, setNotifications, setNotificationCounts])

  useEffect(() => {
    loadData()
  }, [loadData])

  // The address names the open item, so Back and a shared link both land on it.
  const selectedNotification = useMemo(() => notifications.find((n) => n.id === linkedId) ?? null, [linkedId, notifications])
  const closeDetail = useCallback(() => void navigate({ to: '/inbox' }), [navigate])

  const handleAction = async (id: string, action: string) => {
    setLoadingAction(`${action}-${id}`)
    try {
      const res = await executeNotificationAction(id, action)
      if (res.success) {
        updateNotificationState(id, res.new_state as NotificationState)
        if (linkedId === id) closeDetail()
        if (action === 'dismiss' && notifications.find((n) => n.id === id)?.kind === 'value_item') {
          toastUndo(
            t('undo_update_cleared'),
            async () => {
              await restoreNotification(id)
              await loadData()
            },
            t('restored_updates'),
            'undo.inbox_cleared',
          )
        }
      }
    } finally {
      setLoadingAction(null)
    }
  }

  // The dialog answer route resolves the notice too; this mirrors that until the next load.
  const handleAnswered = (id: string, choice: 'approve' | 'decline') => {
    updateNotificationState(id, choice === 'approve' ? 'approved' : 'dismissed')
    if (linkedId === id) closeDetail()
  }

  // Only updates are cleared in bulk; a Needs-you request waits for its own answer.
  const handleClearUpdates = async () => {
    const ids = clearableUpdateIds(notifications)
    try {
      await markAllNotificationsRead()
      await loadData()
      toastUndo(
        tn('updates_cleared', ids.length),
        async () => {
          await restoreNotifications(ids)
          await loadData()
        },
        t('restored_updates'),
        'undo.inbox_cleared',
      )
    } catch (err) {
      toastAlert(failureLine(err, 'fail_clear_updates_newer', t('fail_clear_updates_no_reply')))
    }
  }

  const items = useMemo(() => {
    const q = searchQuery.trim().toLowerCase()
    return notifications.filter(
      (item) => inInboxTab(item, activeTab) && (!q || item.title.toLowerCase().includes(q) || item.description.toLowerCase().includes(q)),
    )
  }, [notifications, activeTab, searchQuery])

  // The server's unread count covers every waiting item; Updates is what is left after the requests.
  const updatesCount =
    activeTab === 'resolved'
      ? Math.max(0, unreadCount - pendingApprovalsCount)
      : notifications.filter((n) => inInboxTab(n, 'all')).length
  const canClearAll = activeTab === 'all' && unreadCount > 0 && clearableUpdateIds(notifications).length > 0

  return (
    <InboxView
      tab={activeTab}
      onTab={(next) => {
        setActiveTab(next)
        if (linkedId) closeDetail()
      }}
      items={items}
      needsCount={pendingApprovalsCount}
      updatesCount={updatesCount}
      query={searchQuery}
      onQuery={setSearchQuery}
      filter={filter}
      onFilter={setFilter}
      selected={selectedNotification}
      checkedAt={checkedAt}
      refreshing={isRefreshing}
      onRefresh={loadData}
      onClearAll={canClearAll ? handleClearUpdates : undefined}
      onAction={handleAction}
      onClear={(id) => handleAction(id, 'dismiss').catch((e) => useHQStore.getState().setSystemNotice(answerErrorLine(e)))}
      onAnswered={handleAnswered}
      loadingAction={loadingAction}
      onOpenLink={(link) => router.history.push(link)}
    />
  )
}
