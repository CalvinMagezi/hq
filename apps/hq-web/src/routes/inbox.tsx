import { NotifyCard } from '~/components/notifications/NotifyCard'
import { answerErrorLine, groupByDay, serverHqLine } from '~/lib/inboxCard'
import { createFileRoute, useParams } from '@tanstack/react-router'
import React, { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import {
  Bell,
  CheckCheck,
  RefreshCw,
  Search,
  Filter,
  FileCode,
  AlertCircle,
  Inbox,
  Loader2,
} from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import {
  fetchNotificationsClient,
  executeNotificationAction,
  markAllNotificationsRead,
  restoreNotification,
  restoreNotifications,
  clearableUpdateIds,
  type NotificationItem,
  type NotificationKind,
  inInboxTab,
} from '~/lib/notificationsApi'
import { toastUndo, toastAlert } from '~/lib/toast'
import { t, tn } from '~/strings'
import { failureLine } from '~/lib/version'
import { NotificationCard } from '~/components/notifications/NotificationCard'
import { NotificationDetailDrawer } from '~/components/notifications/NotificationDetailDrawer'

export const Route = createFileRoute('/inbox')({
  validateSearch: (search: Record<string, unknown>): { tab?: 'updates' | 'done' } => {
    const tab = search.tab === 'updates' || search.tab === 'done' ? search.tab : undefined
    return tab ? { tab } : {}
  },
  component: NotificationsPage,
})

const TAB_OF_SEARCH: Record<'updates' | 'done', TabType> = { updates: 'all', done: 'resolved' }

type TabType = 'pending' | 'all' | 'resolved'
type CategoryType = 'all' | 'value' | 'system'

function NotificationsPage() {
  const { tab } = Route.useSearch()
  const linkedId = useParams({ strict: false }).id
  const [activeTab, setActiveTab] = useState<TabType>(tab ? TAB_OF_SEARCH[tab] : 'pending')
  const [selectedCategory, setSelectedCategory] = useState<CategoryType>('all')
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNotification, setSelectedNotification] = useState<NotificationItem | null>(null)
  const [loadingAction, setLoadingAction] = useState<string | null>(null)
  const [isRefreshing, setIsRefreshing] = useState(false)

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
    } catch (e) {
      console.error('Failed to load notifications:', e)
    } finally {
      setIsRefreshing(false)
    }
  }, [activeTab, setNotifications, setNotificationCounts])

  useEffect(() => {
    loadData()
  }, [loadData])

  // An address with an item opens that item once; closing it must not bring it back on the next refresh.
  const openedLinked = useRef<string | null>(null)
  useEffect(() => {
    if (!linkedId || openedLinked.current === linkedId) return
    const hit = notifications.find((n) => n.id === linkedId)
    if (!hit) return
    openedLinked.current = linkedId
    setSelectedNotification(hit)
  }, [linkedId, notifications])

  const handleAction = async (id: string, action: string) => {
    setLoadingAction(`${action}-${id}`)
    try {
      const res = await executeNotificationAction(id, action)
      if (res.success) {
        updateNotificationState(
          id,
          res.new_state as import('~/lib/notificationsApi').NotificationState
        )
        if (selectedNotification && selectedNotification.id === id) {
          setSelectedNotification(null)
        }
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
    } catch (e) {
      // A refused action (an agent's question approved as a notice, say) must not look like it worked.
      throw e
    } finally {
      setLoadingAction(null)
    }
  }

  // The dialog answer route resolves the notice too; this mirrors that until the next load.
  const handleAnswered = (id: string, choice: 'approve' | 'decline') => {
    updateNotificationState(id, choice === 'approve' ? 'approved' : 'dismissed')
    if (selectedNotification && selectedNotification.id === id) {
      setSelectedNotification(null)
    }
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

  const filteredNotifications = useMemo(() => {
    return notifications.filter((item) => {
      // Tab filter
      if (!inInboxTab(item, activeTab)) return false

      // Category filter
      if (selectedCategory === 'value' && item.kind !== 'value_item') return false
      if (selectedCategory === 'system' && item.kind !== 'action_needed') return false

      // Text search
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase()
        const matchTitle = item.title.toLowerCase().includes(q)
        const matchDesc = item.description.toLowerCase().includes(q)
        if (!matchTitle && !matchDesc) return false
      }

      return true
    })
  }, [notifications, activeTab, selectedCategory, searchQuery])

  return (
    <div className="flex-1 flex flex-col h-full min-w-0 overflow-y-auto overflow-x-hidden px-4 py-6 sm:px-8 max-w-5xl mx-auto w-full">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6">
        <div>
          <div className="flex items-center gap-2.5">
            <div className="p-2 rounded-xl bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">
              <Bell className="w-5 h-5" />
            </div>
            <div>
              <h1 className="text-xl font-bold text-white tracking-tight">{t('inbox_title')}</h1>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 self-start sm:self-auto">
          <button
            type="button"
            onClick={loadData}
            disabled={isRefreshing}
            className="px-3 py-1.5 rounded-xl border border-white/10 text-xs text-neutral-300 hover:text-white hover:bg-white/5 transition-all flex items-center gap-1.5 disabled:opacity-50"
            title="Refresh feed"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isRefreshing ? 'animate-spin' : ''}`} />
            <span className="hidden sm:inline">Refresh</span>
          </button>

          {activeTab === 'pending' && unreadCount > 0 && clearableUpdateIds(notifications).length > 0 && (
            <button
              type="button"
              onClick={handleClearUpdates}
              className="hq-btn-ghost"
            >
              <CheckCheck className="w-3.5 h-3.5 text-emerald-400" />
              <span>{t('inbox_clear_all_updates')}</span>
            </button>
          )}
        </div>
      </div>

      {/* Filter Tabs & Search Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 mb-5">
        {/* Tabs */}
        <div className="flex items-center gap-1 p-1 rounded-xl hq-field self-start max-w-full overflow-x-auto">
          <button
            type="button"
            onClick={() => setActiveTab('pending')}
            className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold transition-all flex items-center gap-1.5 ${
              activeTab === 'pending'
                ? 'hq-seg-on'
                : 'text-neutral-400 hover:text-neutral-200'
            }`}
          >
            <span>{t('inbox_tab_needs')}</span>
            {pendingApprovalsCount > 0 && (
              <span className="px-1.5 py-0.2 rounded-full text-[12px] font-bold text-black" style={{ background: 'var(--warn)' }}>
                {pendingApprovalsCount}
              </span>
            )}
          </button>

          <button
            type="button"
            onClick={() => setActiveTab('all')}
            className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
              activeTab === 'all'
                ? 'bg-white/10 text-white shadow-sm'
                : 'text-neutral-400 hover:text-neutral-200'
            }`}
          >
            {t('inbox_tab_updates')}
          </button>

          <button
            type="button"
            onClick={() => setActiveTab('resolved')}
            className={`px-3.5 py-1.5 rounded-lg text-xs font-semibold transition-all ${
              activeTab === 'resolved'
                ? 'bg-white/10 text-white shadow-sm'
                : 'text-neutral-400 hover:text-neutral-200'
            }`}
          >
            {t('inbox_tab_done')}
          </button>
        </div>

        {/* Search */}
        <div className="relative flex-1 min-w-0 sm:max-w-xs">
          <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-neutral-500" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t('inbox_search')}
            className="w-full pl-8 pr-3 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400 placeholder:text-neutral-600 transition-all"
          />
        </div>
      </div>

      {/* Category Pills */}
      <div className="flex items-center gap-2 flex-wrap mb-5 text-xs ">
        <button
          type="button"
          onClick={() => setSelectedCategory('all')}
          className={`px-3.5 py-1.5 rounded-xl border font-semibold transition-all ${
            selectedCategory === 'all'
              ? 'bg-white/15 text-white border-white/25 shadow-sm'
              : 'text-neutral-400 border-white/10 hover:border-white/20 hover:text-neutral-200'
          }`}
        >
          All Categories
        </button>

        <button
          type="button"
          onClick={() => setSelectedCategory('value')}
          className={`px-3.5 py-1.5 rounded-xl border font-semibold transition-all flex items-center gap-1.5 ${
            selectedCategory === 'value'
              ? 'bg-amber-500/20 text-amber-400 border-amber-500/40 shadow-sm'
              : 'text-neutral-400 border-white/10 hover:border-white/20 hover:text-neutral-200'
          }`}
        >
          <AlertCircle className="w-3.5 h-3.5 text-amber-400" />
          <span>Value Insights</span>
        </button>


        <button
          type="button"
          onClick={() => setSelectedCategory('system')}
          className={`px-3.5 py-1.5 rounded-xl border font-semibold transition-all flex items-center gap-1.5 ${
            selectedCategory === 'system'
              ? 'bg-purple-500/20 text-purple-400 border-purple-500/40 shadow-sm'
              : 'text-neutral-400 border-white/10 hover:border-white/20 hover:text-neutral-200'
          }`}
        >
          <FileCode className="w-3.5 h-3.5 text-purple-400" />
          <span>Code & System</span>
        </button>

      </div>

      {/* Feed List */}
      <div className="space-y-3 pb-6">
        {activeTab === 'pending' && <NotifyCard />}
        {filteredNotifications.length > 0 ? (
          groupByDay(filteredNotifications, Date.now()).map((group) => (
            <section key={group.heading} className="space-y-3">
              {activeTab === 'resolved' && <h2 className="a-h3 px-1">{group.heading}</h2>}
              {group.items.map((notif) => (
                <div key={notif.id} className="space-y-1">
                  {serverHqLine(notif) && <p className="a-muted a-body-sm px-1">{serverHqLine(notif)}</p>}
                  <NotificationCard
                    notification={notif}
                    onSelect={setSelectedNotification}
                    onAction={handleAction}
                    onAnswered={handleAnswered}
                    loadingAction={loadingAction}
                  />
                </div>
              ))}
            </section>
          ))
        ) : (
          <div className="py-16 flex flex-col items-center justify-center text-center p-8 rounded-2xl border border-dashed border-white/10 bg-white/[0.01]">
            <div className="p-4 rounded-2xl bg-white/5 text-neutral-500 mb-3">
              <Inbox className="w-8 h-8" />
            </div>
            <h3 className="text-sm font-bold text-neutral-300">No notifications found</h3>
            <p className="text-xs text-neutral-500 max-w-sm mt-1">
              {activeTab === 'pending'
                ? 'All proposals and suggestions have been approved or dismissed.'
                : 'No notification records match the current filter selection.'}
            </p>
          </div>
        )}
      </div>

      {/* Detail Drawer */}
      <NotificationDetailDrawer
        notification={selectedNotification}
        onClose={() => setSelectedNotification(null)}
        onAction={(id, action) => handleAction(id, action).catch((e) => useHQStore.getState().setSystemNotice(answerErrorLine(e)))}
              onAnswered={handleAnswered}
        loadingAction={loadingAction}
      />
    </div>
  )
}
