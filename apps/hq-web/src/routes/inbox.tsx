import { answerErrorLine } from '~/lib/inboxCard'
import { createFileRoute, useNavigate, useParams, useRouter } from '@tanstack/react-router'
import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useHQStore } from '~/store/hqStore'
import {
  fetchNotificationsClient,
  executeNotificationAction,
  markAllNotificationsRead,
  markNotificationRead,
  clearAllUpdates,
  clearDoneList,
  silenceTopic,
  unsilenceTopic,
  restoreNotification,
  restoreNotifications,
  fetchTrustedSites,
  restoreClearedDialogs,
  forgetTrustedSite,
  clearableUpdateIds,
  inInboxTab,
  type InboxTab,
  type NotificationState,
  type SilenceTopic,
} from '~/lib/notificationsApi'
import { toastUndo, toastAlert, toastNotice } from '~/lib/toast'
import { useWS } from '~/context/WebSocketContext'
import { t, tn } from '~/strings'
import { failureLine } from '~/lib/version'
import { InboxView } from '~/components/notifications/InboxView'
import type { UpdateFilter } from '~/components/notifications/updateKind'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { clearedDialogs, inBatches, pileOf, staleRequests } from '~/lib/catchUp'
import { LATER_MS, neighbour, preferredItem, readLater, splitLater, writeLater, type LaterMap } from '~/lib/inboxView'
import {
  SANDBOX_GRANTS_QUERY_KEY,
  SANDBOX_RULES_QUERY_KEY,
  SANDBOX_SUMMARY_QUERY_KEY,
  isSandboxGrantNotification,
} from '~/lib/sandboxGrantsApi'

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
  // "Later" on an update keeps it out of the list for a few hours, in this browser only.
  const [later, setLater] = useState<LaterMap>(() => readLater())
  const [clock, setClock] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setClock(Date.now()), 60_000)
    return () => window.clearInterval(id)
  }, [])

  const notifications = useHQStore((s) => s.notifications)
  const setNotifications = useHQStore((s) => s.setNotifications)
  const unreadCount = useHQStore((s) => s.unreadNotificationsCount)
  const pendingApprovalsCount = useHQStore((s) => s.pendingApprovalsCount)
  const setNotificationCounts = useHQStore((s) => s.setNotificationCounts)
  const updateNotificationState = useHQStore((s) => s.updateNotificationState)
  const markRead = useHQStore((s) => s.markNotificationRead)

  // Reloads overlap (an event during a tab switch): only the newest answer is shown.
  const loadSeq = useRef(0)
  const loadData = useCallback(async () => {
    const seq = ++loadSeq.current
    setIsRefreshing(true)
    try {
      const done = activeTab === 'resolved'
      const res = await fetchNotificationsClient({ state: done ? 'resolved' : 'pending' })
      if (seq !== loadSeq.current) return
      setNotifications(res.notifications)
      // The Done list has nothing pending in it, so its counts would zero the badge.
      if (!done) setNotificationCounts(res.unread_count, res.pending_approvals_count)
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

  // The list follows HQ while it is open: a request that arrives, or one answered in a chat or
  // on another device, shows here without Refresh. A reconnect and coming back to the tab read
  // again, since events sent meanwhile are gone.
  const { subscribe, connected } = useWS()
  useEffect(() => {
    if (connected) void loadData()
  }, [connected, loadData])
  useEffect(
    () =>
      subscribe((msg) => {
        if (msg.type === 'inbox:change' || msg.type === 'notification:resolved' || msg.type === 'notification:restored') void loadData()
      }),
    [subscribe, loadData],
  )
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void loadData()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [loadData])

  // The address names the open item, so Back and a shared link both land on it.
  const selectedNotification = useMemo(() => notifications.find((n) => n.id === linkedId) ?? null, [linkedId, notifications])
  const closeDetail = useCallback(() => void navigate({ to: '/inbox' }), [navigate])

  // Opening an item reads it.
  const openUnread = selectedNotification && selectedNotification.read === false && selectedNotification.state === 'pending' ? selectedNotification.id : null
  useEffect(() => {
    if (!openUnread) return
    markRead(openUnread)
    markNotificationRead(openUnread).catch(() => undefined)
  }, [openUnread, markRead])

  const queryClient = useQueryClient()

  const handleAction = async (id: string, action: string, opts?: { starter?: string }) => {
    setLoadingAction(`${action}-${id}`)
    try {
      const res = await executeNotificationAction(id, action, opts)
      if (res.success) {
        updateNotificationState(id, res.new_state as NotificationState)
        if (linkedId === id) closeDetail()
        const target = notifications.find((n) => n.id === id)
        if (target && isSandboxGrantNotification(target)) {
          void queryClient.invalidateQueries({ queryKey: SANDBOX_GRANTS_QUERY_KEY })
          void queryClient.invalidateQueries({ queryKey: SANDBOX_RULES_QUERY_KEY })
          void queryClient.invalidateQueries({ queryKey: SANDBOX_SUMMARY_QUERY_KEY })
        }
        if (action === 'dismiss' && target?.kind === 'value_item') {
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
    try {
      const ids = (await clearAllUpdates()).ids ?? clearableUpdateIds(notifications)
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

  const failed = () => toastAlert(t('inbox_fail_no_reply'))
  const handleReadAll = () => markAllNotificationsRead().then(loadData).catch(failed)
  const handleClearDone = () =>
    clearDoneList()
      .then(loadData)
      .then(() => toastNotice(t('inbox_done_cleared')))
      .catch(failed)
  // Settings keeps its own copy of the silences; a change made here makes it read again.
  const forgetSilences = () => queryClient.invalidateQueries({ queryKey: ['inbox-silences'] })
  // Silencing is about what interrupts: the items stay listed. Undo turns the topic back on.
  const handleSilence = (topic: SilenceTopic) =>
    silenceTopic(topic)
      .then(loadData)
      .then(() => toastUndo(t('silence_done'), () => unsilenceTopic(topic).then(loadData).then(forgetSilences), t('silence_undone')))
      .then(forgetSilences)
      .catch(failed)

  const matching = useMemo(() => {
    const q = searchQuery.trim().toLowerCase()
    return notifications.filter(
      (item) => inInboxTab(item, activeTab) && (!q || item.title.toLowerCase().includes(q) || item.description.toLowerCase().includes(q)),
    )
  }, [notifications, activeTab, searchQuery])
  // Updates that wait for later leave the list; a request never does, since an approval has no Later.
  const { waiting, rest } = useMemo(
    () => (activeTab === 'all' ? splitLater(matching, later, clock) : { waiting: [], rest: matching }),
    [matching, later, clock, activeTab],
  )

  const sendLater = (id: string) => {
    const next = { ...later, [id]: Date.now() + LATER_MS }
    setLater(next)
    writeLater(next)
    if (linkedId === id) closeDetail()
    toastUndo(t('inbox_later_done'), async () => {
      const { [id]: _gone, ...back } = next
      setLater(back)
      writeLater(back)
    }, t('inbox_bring_back'))
  }
  const bringBack = (id: string) => {
    const { [id]: _gone, ...back } = later
    setLater(back)
    writeLater(back)
  }

  // The catch-up card: past five waiting requests, the ones from helpers that are done can be cleared together.
  const helperSessions = useQuery({ queryKey: ['search', 'helpers'], queryFn: () => globalSessionsApi.list({}), enabled: activeTab === 'pending', staleTime: 30_000, retry: false })
  const [onlyNeeds, setOnlyNeeds] = useState(false)
  const [clearing, setClearing] = useState(false)
  const waitingRequests = notifications.filter((n) => inInboxTab(n, 'pending') && n.state === 'pending')
  // Without an answer about the helpers nothing is called old, so nothing is offered for clearing by mistake.
  const stale = useMemo(() => (helperSessions.data ? staleRequests(waitingRequests, helperSessions.data) : []), [waitingRequests, helperSessions.data])
  const pile = activeTab === 'pending' ? pileOf(waitingRequests, stale) : null
  // "Show the ones that need you" sets the old ones aside without clearing them.
  const staleIds = useMemo(() => new Set(stale.map((n) => n.id)), [stale])
  const items = onlyNeeds && pile ? rest.filter((n) => !staleIds.has(n.id)) : rest
  const clearOld = async () => {
    setClearing(true)
    try {
      const cleared = await inBatches(stale, 6, (n) => executeNotificationAction(n.id, 'dismiss'))
      await loadData()
      if (cleared.length > 0) {
        const ids = cleared.map((n) => n.id)
        toastUndo(
          tn('inbox_pile_cleared', ids.length),
          async () => {
            await restoreClearedDialogs(ids)
            await loadData()
          },
          t('restored_updates'),
        )
      }
      if (cleared.length < stale.length) toastAlert(t('inbox_pile_failed'))
    } finally {
      setClearing(false)
    }
  }
  const restoreGone = (ids: string[]) =>
    restoreClearedDialogs(ids)
      .then(loadData)
      .then(() => toastNotice(tn('inbox_gone_restored', ids.length)))
      .catch(failed)

  const sites = useQuery({ queryKey: ['inbox-sites'], queryFn: fetchTrustedSites, enabled: activeTab === 'resolved', staleTime: 30_000 })
  const forgetSite = (site: string) =>
    forgetTrustedSite(site)
      .then(() => queryClient.invalidateQueries({ queryKey: ['inbox-sites'] }))
      .then(() => toastNotice(t('inbox_site_removed')))
      .catch(failed)

  // On a wide screen the card is never empty while something waits: the sign-in, or the oldest request, opens by itself.
  // Answering one closes it, and the next opens.
  const [wide, setWide] = useState(false)
  useEffect(() => {
    const query = window.matchMedia('(min-width: 1024px)')
    const update = () => setWide(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])
  const autoOpen = wide && activeTab === 'pending' && !linkedId && checkedAt !== null && searchQuery.trim() === '' ? preferredItem(items) : null
  useEffect(() => {
    if (autoOpen) void navigate({ to: '/inbox/$id', params: { id: autoOpen.id }, replace: true })
  }, [autoOpen, navigate])

  // Keys: J and K move, Esc goes back, E clears an update, H says Later, Z undoes the last clear.
  // An approval has no single-key answer on purpose; answering takes a deliberate click.
  const lastCleared = useRef<string | null>(null)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null
      if (e.ctrlKey || e.metaKey || e.altKey || (el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)))) return
      const open = items.find((n) => n.id === linkedId)
      const go = (step: 1 | -1) => {
        const id = neighbour(items, linkedId, step)
        if (id) void navigate({ to: '/inbox/$id', params: { id } })
      }
      if (e.key === 'j' || e.key === 'ArrowDown') go(1)
      else if (e.key === 'k' || e.key === 'ArrowUp') go(-1)
      else if (e.key === 'Escape' && linkedId) closeDetail()
      else if (e.key === 'e' && open && open.kind === 'value_item' && open.state === 'pending') {
        lastCleared.current = open.id
        void handleAction(open.id, 'dismiss')
      } else if (e.key === 'h' && open && open.kind === 'value_item' && open.state === 'pending') sendLater(open.id)
      else if (e.key === 'z' && lastCleared.current) {
        const id = lastCleared.current
        lastCleared.current = null
        void restoreNotification(id).then(loadData)
      } else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

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
      onReadAll={activeTab === 'all' && notifications.some((n) => inInboxTab(n, 'all') && n.read === false) ? handleReadAll : undefined}
      onClearDone={activeTab === 'resolved' && items.length > 0 ? handleClearDone : undefined}
      onSilence={handleSilence}
      later={waiting}
      pile={pile}
      pileBusy={clearing}
      onClearOld={() => void clearOld()}
      onlyNeeds={onlyNeeds}
      onToggleNeeds={() => setOnlyNeeds((v) => !v)}
      cleared={activeTab === 'resolved' ? clearedDialogs(notifications) : undefined}
      onRestore={(ids) => void restoreGone(ids)}
      onLater={sendLater}
      onBringBack={bringBack}
      sites={sites.data?.sites}
      onForgetSite={(site) => void forgetSite(site)}
      onAction={handleAction}
      onClear={(id) => handleAction(id, 'dismiss').catch((e) => useHQStore.getState().setSystemNotice(answerErrorLine(e)))}
      onAnswered={handleAnswered}
      loadingAction={loadingAction}
      onOpenLink={(link) => router.history.push(link)}
    />
  )
}
