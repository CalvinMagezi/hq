import { HqHttpError } from '~/lib/hqAuth'
import { createFileRoute, useParams, useRouterState } from '@tanstack/react-router'
import { useState, useMemo, useEffect, useRef } from 'react'
import { ClipboardList, Filter, ListChecks, Plus, Search, List, Columns3, GanttChart, X, AlertTriangle, Info, ChevronDown, Eye } from 'lucide-react'
import {
  createTaskClient,
  updateTaskClient,
  deleteTaskClient,
  STATUS_LABELS,
  STATUS_ORDER,
  HTTP_CONFLICT,
  fetchRecentWorkSessionsClient,
  fetchStaleTasksClient,
  fetchTasksClient,
  restoreTaskClient,
  type Initiative,
  type TaskItem,
  type TaskPriority,
  type WorkSession,
  type TaskStatus,
  type TaskUpdateResult,
  type UpdateTaskInput,
} from '~/lib/tasksApi'
import { toastUndo, toastAlert } from '~/lib/toast'
import { Dialog } from '~/components/ui/Dialog'
import { t, tn } from '~/strings'
import { dependentsOf, doneQuestion, type DoneQuestion } from '~/lib/doneAsk'
import { fetchTaskVerification } from '~/lib/verificationApi'
import { TaskListView, type ListSections } from '~/components/tasks/TaskListView'
import { BulkActionBar } from '~/components/tasks/BulkActionBar'
import { bulkMessage, failureNotice, runBulk, splitUnchanged, uniqueWarnings } from '~/components/tasks/bulk'
import { useTaskSelection } from '~/components/tasks/useTaskSelection'
import { withoutSections } from '~/components/tasks/hierarchy'
import { OutboxGroup } from '~/components/chat/OutboxGroup'
import { titleOf, useTaskOutbox } from '~/components/tasks/useTaskOutbox'
import { newClientId } from '~/lib/outbox'
import { usePersistedState } from '~/lib/usePersistedState'
import { CHIPS, TABS, chipStatus, matchesTab, type TaskChip, type TaskTab } from '~/lib/tasksFilters'
import { formatDay, todayDay } from '~/components/tasks/dates'
import { TaskBoardView } from '~/components/tasks/TaskBoardView'
import { TaskGanttView } from '~/components/tasks/TaskGanttView'
import { TaskDetailMain, TaskDetailSide } from '~/components/tasks/TaskDetailDrawer'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { useEdition } from '~/lib/edition'
import { useSharedConnection } from '~/lib/connection'
import { parseSqliteUtc } from '~/lib/tasksApi'
import '~/components/tasks/tasks.css'
import { useBackClosesLayer } from '~/lib/backLayer'
import { TaskFormModal } from '~/components/tasks/TaskFormModal'
import { ALL_SELECTION, TasksSidebar, type TaskSelection } from '~/components/tasks/TasksSidebar'
import { useTasksData } from '~/components/tasks/useTasksData'
import { usePolled } from '~/components/sessions/usePolled'
import { StaleIdsContext, stableSet } from '~/components/tasks/staleContext'
import { WorkingNowContext, workingNow } from '~/components/tasks/workingContext'
import { WorkingNowPanel } from '~/components/tasks/WorkingNowPanel'
import { InitiativeBriefing } from '~/components/tasks/InitiativeBriefing'
import { VerificationBar } from '~/components/tasks/verification/VerificationBar'
import { AwayPage } from '~/components/tasks/AwayPage'
import { AwayLinkRow } from '~/components/tasks/TasksBody'
import { SweepSheet } from '~/components/tasks/verification/SweepSheet'
import { AuditSheet } from '~/components/tasks/verification/AuditSheet'

type TasksView = 'list' | 'board' | 'timeline'
const VIEW_KEY = 'hq.tasks.view'
const isTasksView = (v: unknown): v is TasksView => v === 'list' || v === 'board' || v === 'timeline'

const VIEWS: { id: TasksView; label: string; icon: typeof List }[] = [
  { id: 'list', label: t('tasks_field_list'), icon: List },
  { id: 'board', label: t('web_board'), icon: Columns3 },
  { id: 'timeline', label: t('web_timeline'), icon: GanttChart },
]

/** How far back the timeline draws actual work, and how often it refreshes. */
const TIMELINE_WORK_DAYS = 60
const TIMELINE_WORK_POLL_MS = 60_000
/** How often the archived list and the stale ids refresh while shown. */
const ARCHIVED_POLL_MS = 60_000
const STALE_POLL_MS = 120_000
/** Who is working right now: a short window and a quick poll, since a lease can start or end any minute. */
const LIVE_WORK_DAYS = 1
const LIVE_WORK_POLL_MS = 120_000
const NO_STALE: ReadonlySet<string> = new Set()
const NO_WORKING: ReadonlyMap<string, string> = new Map()
const NO_SESSIONS: readonly WorkSession[] = []

export const Route = createFileRoute('/tasks')({
  validateSearch: (search: Record<string, unknown>): { view?: TasksView; task?: string } => {
    const view = VIEWS.find((v) => v.id === search.view)?.id
    const task = typeof search.task === 'string' && search.task ? search.task : undefined
    return { ...(view && view !== 'list' ? { view } : {}), ...(task ? { task } : {}) }
  },
  component: TasksPage,
})

type StatusTab = 'active' | 'all' | 'complete' | 'archived'

const FOLDED_SECTIONS_KEY = 'hq-tasks-collapsed-sections'
const isStringArray = (v: unknown) => Array.isArray(v) && v.every((x) => typeof x === 'string')

const errorMessage = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback)

interface TaskFilters {
  statusTab: StatusTab
  statusFilter: TaskStatus | 'all'
  tab: TaskTab
  tagFilter: string
  selection: TaskSelection
  searchQuery: string
  view: TasksView
}

function matchesSearch(t: TaskItem, query: string): boolean {
  const q = query.toLowerCase()
  return (
    t.title.toLowerCase().includes(q) ||
    t.description.toLowerCase().includes(q) ||
    t.display_id.toLowerCase().includes(q) ||
    t.tags.some((tag) => tag.toLowerCase().includes(q))
  )
}

function matchesSelection(t: TaskItem, selection: TaskSelection, initiativeById: Map<string, Initiative>): boolean {
  if (selection.initiativeId) return t.initiative_id === selection.initiativeId
  if (selection.folderId) return initiativeById.get(t.initiative_id)?.folder_id === selection.folderId
  if (selection.spaceId) return initiativeById.get(t.initiative_id)?.space_id === selection.spaceId
  return true
}

function filterTasks(tasks: TaskItem[], f: TaskFilters, initiativeById: Map<string, Initiative>): TaskItem[] {
  return tasks.filter((t) => {
    // A specific status is more precise than the coarse Active/All/Complete
    // tab, so it takes over entirely rather than being ANDed with the tab
    // (picking "Ready for Review" while the tab sits on "Complete" would
    // otherwise always produce zero results).
    if (f.statusFilter !== 'all') {
      if (t.status !== f.statusFilter) return false
    } else if (f.view !== 'board') {
      if (f.tab === 'today' && f.statusTab === 'active' && isDoneToday(t)) return matchesFilters(t, f, initiativeById)
      // The board shows every status as a column, so the coarse tab is ignored there.
      if (f.statusTab === 'active' && t.status === 'complete') return false
      if (f.statusTab === 'complete' && t.status !== 'complete') return false
      if (!matchesTab(t, f.tab, formatDay(todayDay()))) return false
    }
    return matchesFilters(t, f, initiativeById)
  })
}

function matchesFilters(t: TaskItem, f: TaskFilters, initiativeById: Map<string, Initiative>): boolean {
  if (f.tagFilter !== 'all' && !t.tags.includes(f.tagFilter)) return false
  if (!matchesSelection(t, f.selection, initiativeById)) return false
  return !f.searchQuery.trim() || matchesSearch(t, f.searchQuery)
}

/** The Today tab keeps what was finished today, in its own folded group. */
function isDoneToday(t: TaskItem): boolean {
  return t.status === 'complete' && t.completed_at !== null && parseSqliteUtc(t.completed_at).toDateString() === new Date().toDateString()
}

function TasksPage() {
  const [rememberedView, setRememberedView] = usePersistedState<TasksView>(VIEW_KEY, 'list', isTasksView)
  const view: TasksView = Route.useSearch().view ?? rememberedView
  const navigate = Route.useNavigate()
  const [notice, setNotice] = useState<string | null>(null)
  const [tab, setTab] = useState<TaskTab>('all')
  const [chip, setChip] = useState<TaskChip>('active')
  const onAll = tab === 'all' && view !== 'board'
  const statusTab: StatusTab = onAll && chip === 'archived' ? 'archived' : onAll && chip === 'done' ? 'complete' : 'active'
  const statusFilter: TaskStatus | 'all' = (onAll && chipStatus(chip)) || 'all'
  const [tagFilter, setTagFilter] = useState<string>('all')
  const [selection, setSelection] = useState<TaskSelection>(ALL_SELECTION)
  const [checkingOpen, setCheckingOpen] = useState(false)
  const [listsOpen, setListsOpen] = useState(false)
  const [viewMenu, setViewMenu] = useState(false)
  const [sideOpen, setSideOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  // The initiative whose briefing is open; picking another initiative closes it.
  const [briefingFor, setBriefingFor] = useState<string | null>(null)
  const [formOpen, setFormOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  // Task verification: the sweep sheet ("Verify now") and the audit stack.
  const [sweepOpen, setSweepOpen] = useState(false)
  const [auditOpen, setAuditOpen] = useState(false)

  const {
    tasks,
    upsertTask,
    removeTask,
    spaces,
    folders,
    initiatives,
    selectedTaskId,
    setSelectedTaskId,
    isRefreshing,
    loadTaxonomy,
    loadTasks,
  } = useTasksData()

  // Actual work, drawn under the plan; only fetched while the timeline is showing.
  const recentWork = usePolled(
    'timeline-work',
    async () => (await fetchRecentWorkSessionsClient(TIMELINE_WORK_DAYS)).work_sessions,
    TIMELINE_WORK_POLL_MS,
    view === 'timeline',
    true,
    ['task:sync']
  )

  const live = usePolled(
    'live-work',
    async () => (await fetchRecentWorkSessionsClient(LIVE_WORK_DAYS)).work_sessions.filter((s) => s.ended_at === null),
    LIVE_WORK_POLL_MS,
    true,
    true,
    ['task:sync']
  )

  const archived = usePolled(
    'archived-tasks',
    async () => (await fetchTasksClient({ archived: true })).tasks,
    ARCHIVED_POLL_MS,
    statusTab === 'archived',
    true,
    ['task:sync']
  )
  const staleSeen = useRef<ReadonlySet<string>>(NO_STALE)
  const stale = usePolled(
    'stale-task-ids',
    async () => {
      const next = new Set((await fetchStaleTasksClient()).tasks.map((s) => s.task_id))
      staleSeen.current = stableSet(next, staleSeen.current)
      return staleSeen.current
    },
    STALE_POLL_MS,
    true,
    true,
    ['task:sync']
  )
  const archivedTasks = archived.data ?? []

  const { task: linkedSearchId } = Route.useSearch()
  const edition = useEdition()
  const lite = edition === 'lite'
  const connection = useSharedConnection()
  const away = useRouterState({ select: (s) => s.location.pathname }) === '/tasks/away'
  const linkedParam = useParams({ strict: false }).id
  const linkedTaskId = linkedParam ?? linkedSearchId
  useEffect(() => {
    if (linkedTaskId) setSelectedTaskId(linkedTaskId)
  }, [linkedTaskId, setSelectedTaskId])

  const selectedTask = useMemo(
    () => tasks.find((t) => t.id === selectedTaskId) ?? archivedTasks.find((t) => t.id === selectedTaskId) ?? null,
    [tasks, archivedTasks, selectedTaskId]
  )

  // Inside the Android app, Back closes the task drawer and the new-task form before it leaves the page.
  useBackClosesLayer(selectedTask != null, () => setSelectedTaskId(null))
  useBackClosesLayer(formOpen, () => setFormOpen(false))

  const liveSessions = live.data ?? NO_SESSIONS
  const workingMap = useMemo(() => (liveSessions.length ? workingNow(liveSessions) : NO_WORKING), [liveSessions])
  const taskById = useMemo(() => new Map(tasks.map((t) => [t.id, t])), [tasks])

  const initiativeById = useMemo(() => new Map(initiatives.map((i) => [i.id, i])), [initiatives])

  const allTags = useMemo(() => {
    const tags = new Set<string>()
    for (const t of tasks) for (const tag of t.tags) tags.add(tag)
    return Array.from(tags).sort((a, b) => a.localeCompare(b))
  }, [tasks])

  const filtered = useMemo(
    () =>
      filterTasks(
        statusTab === 'archived' ? archivedTasks : tasks,
        { statusTab, statusFilter, tab, tagFilter, selection, searchQuery, view },
        initiativeById
      ),
    [tasks, archivedTasks, statusTab, statusFilter, tab, tagFilter, selection, initiativeById, searchQuery, view]
  )

  // Initiative sections the list folds away. Kept in the browser, so a section skipped once stays skipped.
  const [foldedNames, setFoldedNames] = usePersistedState<string[]>(FOLDED_SECTIONS_KEY, [], isStringArray)
  const sections = useMemo<ListSections>(
    () => ({
      collapsed: new Set(foldedNames),
      onToggle: (name) => setFoldedNames((prev) => (prev.includes(name) ? prev.filter((n) => n !== name) : [...prev, name])),
      onSetMany: (names, folded) =>
        setFoldedNames((prev) => (folded ? [...new Set([...prev, ...names])] : prev.filter((n) => !names.includes(n)))),
    }),
    [foldedNames, setFoldedNames]
  )

  // What the list actually shows: a folded section hides its tasks, so they are neither counted nor
  // ticked by "Select all", and folding a section un-ticks what was ticked in it.
  const onScreen = useMemo(
    () => withoutSections(filtered, sections.collapsed, initiativeById),
    [filtered, sections, initiativeById]
  )

  // Picking tasks for a bulk action only makes sense in the list, and not in the archive.
  const picking = useTaskSelection(onScreen, view === 'list' && statusTab !== 'archived')
  const [bulkProgress, setBulkProgress] = useState<{ settled: number; total: number } | null>(null)
  const [bulkResult, setBulkResult] = useState<string | null>(null)

  const setView = (next: TasksView) => {
    setRememberedView(next)
    navigate({ search: { view: next }, replace: true })
  }

  const showWarnings = (result: TaskUpdateResult) => {
    if (result.warnings?.length) setNotice(result.warnings.join(' '))
  }

  const taskOutbox = useTaskOutbox(upsertTask)

  const handleCreate = async (input: Parameters<typeof createTaskClient>[0]) => {
    const withId = { ...input, client_id: newClientId() }
    try {
      upsertTask(await createTaskClient(withId))
    } catch (e) {
      // Only a lost connection is kept for later; a refusal from HQ is shown to the person.
      if (!(e instanceof TypeError) || !taskOutbox.enabled) throw e
      const refusal = await taskOutbox.queue(withId)
      if (refusal) throw new Error(refusal)
      return
    }
    await loadTaxonomy()
  }

  const [bulkDonePending, setBulkDonePending] = useState<TaskItem[] | null>(null)
  const [doneAsk, setDoneAsk] = useState<{ id: string; patch: Record<string, unknown>; why: Exclude<DoneQuestion, null> } | null>(null)

  const withBusy = async (fn: () => Promise<void>, fallback: string) => {
    setBusy(true)
    try {
      await fn()
    } catch (e) {
      console.error(`${fallback}:`, e)
      setNotice(errorMessage(e, fallback))
    } finally {
      setBusy(false)
    }
  }

  const handleUpdate = async (id: string, patch: Record<string, unknown>) => {
    const previous = taskById.get(id)
    let checkPending = false
    if (patch.status === 'complete' && previous && previous.status !== 'complete') {
      // A check that cannot be looked up does not block the person from marking it done.
      checkPending = await fetchTaskVerification(id).then((v) => v.waiting_for?.who === 'verifier', () => false)
    }
    const why = doneQuestion(previous, patch, taskById.values(), checkPending)
    if (why) {
      setDoneAsk({ id, patch, why })
      return
    }
    return applyUpdate(id, patch)
  }

  const applyUpdate = (id: string, patch: Record<string, unknown>) =>
    withBusy(async () => {
      const previous = taskById.get(id)
      const task = await updateTaskClient(id, patch)
      upsertTask(task)
      showWarnings(task)
      if (patch.status === 'complete' && previous && previous.status !== 'complete') {
        toastUndo(
          t('undo_done_task', { title: task.title }),
          async () => upsertTask(await updateTaskClient(id, { status: previous.status, expected_status: 'complete' })),
          t('restored_task'),
          'undo.task_done',
        )
      }
    }, t('tasks_update_failed'))

  // Optimistic writes for drag-and-drop: show the change at once, revert if the server refuses it.
  const applyOptimistic = async (task: TaskItem, patch: UpdateTaskInput, conflictMessage: string) => {
    upsertTask({ ...task, ...patch } as TaskItem)
    try {
      const updated = await updateTaskClient(task.id, patch)
      upsertTask(updated)
      showWarnings(updated)
    } catch (e) {
      upsertTask(task)
      const conflict = e instanceof HqHttpError && e.status === HTTP_CONFLICT
      setNotice(conflict ? conflictMessage : errorMessage(e, t('tasks_update_failed')))
      if (conflict) loadTasks()
    }
  }

  const handleStatusMove = async (task: TaskItem, status: TaskStatus) => {
    // Done asks first when others wait on the task or a check is pending, and offers its own Undo.
    if (status === 'complete') return handleUpdate(task.id, { status, expected_status: task.status })
    await applyOptimistic(task, { status, expected_status: task.status }, t('tasks_moved_by_other', { id: task.display_id }))
    toastUndo(
      t('tasks_moved_to', { status: STATUS_LABELS[status] }),
      async () => upsertTask(await updateTaskClient(task.id, { status: task.status, expected_status: status })),
      t('restored_task'),
    )
  }

  const handleReschedule = (task: TaskItem, patch: UpdateTaskInput) =>
    applyOptimistic(task, patch, t('tasks_timeline_changed', { id: task.display_id }))

  const handleCreateSubtask = async (parent: TaskItem, title: string) => {
    try {
      const task = await createTaskClient({ title, parent_task_id: parent.id, created_by: 'operator' })
      upsertTask(task)
    } catch (e) {
      setNotice(errorMessage(e, t('tasks_sub_add_failed')))
    }
  }

  const handleDelete = (id: string, cascade: boolean, purge = false) =>
    withBusy(async () => {
      const res = await deleteTaskClient(id, cascade, purge)
      for (const deletedId of res.deleted_ids ?? [id]) removeTask(deletedId)
      setSelectedTaskId(null)
      if (purge) {
        await archived.refresh()
        return
      }
      toastUndo(
        t('undo_task_archived'),
        async () => {
          upsertTask(await restoreTaskClient(id))
          await archived.refresh()
        },
        t('restored_task'),
        'undo.task_archived',
      )
    }, t('fail_archive_task_no_reply'))

  const handleRestore = (id: string) =>
    withBusy(async () => {
      upsertTask(await restoreTaskClient(id))
      await archived.refresh()
    }, t('tasks_restore_failed'))

  // One PATCH per task, a few at a time: each keeps its own checks, notifications and
  // soft-dependency warnings, and one task failing leaves the rest done.
  const runBulkUpdate = async (
    targets: TaskItem[],
    unchanged: number,
    patchFor: (task: TaskItem) => UpdateTaskInput,
    phrase: string
  ): Promise<TaskItem[]> => {
    setNotice(null)
    if (targets.length === 0) {
      setBulkResult(bulkMessage(0, 0, unchanged, phrase))
      return []
    }
    setBulkResult(null)
    setBulkProgress({ settled: 0, total: targets.length })
    const outcome = await runBulk(
      targets,
      async (task) => {
        const updated = await updateTaskClient(task.id, patchFor(task))
        upsertTask(updated)
        return updated
      },
      (settled, total) => setBulkProgress({ settled, total })
    )
    setBulkProgress(null)
    // Done ones are un-ticked; a failed one stays ticked so it can be tried again.
    picking.setMany(outcome.done.map((d) => d.task), false)
    setBulkResult(bulkMessage(outcome.done.length, targets.length, unchanged, phrase))
    const problems = [failureNotice(outcome.failed), ...uniqueWarnings(outcome.done.map((d) => d.value.warnings))]
    if (problems.some(Boolean)) setNotice(problems.filter(Boolean).join(' '))
    if (outcome.failed.some((f) => f.error instanceof HqHttpError && f.error.status === HTTP_CONFLICT)) loadTasks()
    return outcome.done.map((d) => d.task)
  }

  // Marking many tasks done asks first (others may wait on them), then offers Undo for all of them.
  const confirmBulkDone = async () => {
    const targets = bulkDonePending ?? []
    setBulkDonePending(null)
    const before = new Map(targets.map((task) => [task.id, task.status]))
    const done = await runBulkUpdate(
      targets,
      0,
      (task) => ({ status: 'complete', expected_status: task.status }),
      t('tasks_bulk_phrase_done', { status: STATUS_LABELS.complete })
    )
    if (done.length === 0) return
    toastUndo(
      t('bulk_done_line', { n: done.length }),
      async () => {
        const failed: TaskItem[] = []
        for (const task of done) {
          try {
            upsertTask(await updateTaskClient(task.id, { status: before.get(task.id) ?? 'to_do', expected_status: 'complete' }))
          } catch {
            failed.push(task)
          }
        }
        if (failed.length > 0) {
          throw new Error(t('fail_change_some_tasks', { done: done.length - failed.length, task: failed[0].title }))
        }
      },
      t('restored_task'),
      'undo.task_done',
    )
  }

  const handleBulkStatus = (status: TaskStatus) => {
    const { toChange, unchanged } = splitUnchanged(picking.selected, (t) => t.status === status)
    if (status === 'complete' && toChange.length > 0) {
      setBulkDonePending(toChange)
      return Promise.resolve()
    }
    return runBulkUpdate(
      toChange,
      unchanged,
      (task) => ({ status, expected_status: task.status }),
      `moved to ${STATUS_LABELS[status]}`
    )
  }

  const handleBulkPriority = (priority: TaskPriority | null) => {
    const { toChange, unchanged } = splitUnchanged(picking.selected, (t) => t.priority === priority)
    return runBulkUpdate(toChange, unchanged, () => ({ priority }), `set to ${priority ?? 'no'} priority`)
  }

  // Any change to what is ticked retires the last result line.
  const afterPick = <A extends unknown[]>(fn: (...args: A) => void) => (...args: A) => {
    setBulkResult(null)
    fn(...args)
  }

  const activeCount = tasks.filter((t) => t.status !== 'complete').length
  const todayStr = formatDay(todayDay())
  const tabCounts: Record<TaskTab, number> = {
    today: tasks.filter((x) => x.status !== 'complete' && matchesTab(x, 'today', todayStr)).length,
    upcoming: tasks.filter((x) => x.status !== 'complete' && matchesTab(x, 'upcoming', todayStr)).length,
    all: activeCount,
  }
  const readyCount = tasks.filter((x) => x.status === 'ready_for_review').length
  const unreachable = connection === 'unreachable' || connection === 'offline'
  const canSelect = view === 'list' && statusTab !== 'archived'
  const detailOpen = selectedTask != null || away
  const closeDetail = () => {
    setSideOpen(false)
    if (away) void navigate({ to: '/tasks' })
    else setSelectedTaskId(null)
  }
  const openTask = (id: string | null) => setSelectedTaskId(id)
  // Board and Timeline are the detail pane themselves; a task opened from them replaces them until closed.
  const wideView = view !== 'list'
  // Check it opens the task, where its check is on top. Lite has no checking.
  const checkIt = lite ? undefined : (task: TaskItem) => openTask(task.id)
  const markDone = (task: TaskItem) =>
    void handleUpdate(task.id, { status: task.status === 'complete' ? 'to_do' : 'complete', expected_status: task.status })

  const showAs = (
    <div className="tk-menu-at">
      <button type="button" className="a-ib tk-viewmenu" aria-haspopup="menu" aria-expanded={viewMenu} aria-label={t('tasks_show_as')} onClick={() => setViewMenu((v) => !v)}>
        <Eye className="a-ic" aria-hidden="true" />
      </button>
      {viewMenu && (
        <div role="menu" className="a-menu tk-pop">
          {VIEWS.map(({ id, label, icon: Icon }) => (
            <button key={id} type="button" role="menuitemradio" aria-checked={view === id} className="a-menu-item" onClick={() => { setViewMenu(false); setView(id) }}>
              <Icon className="a-ic sm" aria-hidden="true" />
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  )

  const headButtons = (
    <>
      <div className="a-seg tk-viewsw" role="group" aria-label={t('tasks_see_as')}>
        {VIEWS.map(({ id, label, icon: Icon }) => (
          <button key={id} type="button" className="a-seg-item" aria-pressed={view === id} onClick={() => setView(id)}>
            <Icon className="a-ic xs" aria-hidden="true" />
            {label}
          </button>
        ))}
      </div>
      {showAs}
      {canSelect && (
        <button
          type="button"
          className="a-ib"
          aria-pressed={picking.active}
          aria-label={t('tasks_pick_several')}
          title={t('tasks_pick_several_title')}
          onClick={() => {
            setBulkResult(null)
            if (picking.active) picking.stop()
            else picking.start()
          }}
        >
          <ListChecks className="a-ic" aria-hidden="true" />
        </button>
      )}
      <button type="button" className="a-ib" aria-label={t('tasks_filter_by_list')} onClick={() => setListsOpen(true)}>
        <Filter className="a-ic" aria-hidden="true" />
      </button>
      <button type="button" className="a-btn compact tk-new" onClick={() => setFormOpen(true)}>
        <Plus className="a-ic sm" aria-hidden="true" />
        {t('tasks_new_task')}
      </button>
    </>
  )

  const controls = (
    <div className="tk-bar">
      <label className="a-search">
        <Search className="a-ic sm" aria-hidden="true" />
        <input type="search" value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} placeholder={t('tasks_find')} aria-label={t('tasks_find')} />
      </label>
      <select value={tagFilter} onChange={(e) => setTagFilter(e.target.value)} aria-label={t('tasks_filter_by_tag')} className="a-input">
        <option value="all">{t('tasks_all_tags')}</option>
        {allTags.map((tag) => (
          <option key={tag} value={tag}>
            {tag}
          </option>
        ))}
      </select>
    </div>
  )

  const tabs = picking.active ? (
    <BulkActionBar
      total={onScreen.length}
      ticked={picking.selected.length}
      progress={bulkProgress}
      message={bulkResult}
      onSelectAll={afterPick(() => picking.setMany(onScreen, true))}
      onClear={afterPick(picking.clear)}
      onDone={afterPick(picking.stop)}
      onStatus={handleBulkStatus}
      onPriority={handleBulkPriority}
    />
  ) : (
    <div className="a-tabs fit" role="tablist" aria-label={t('place_tasks')}>
      {TABS.map((id) => (
        <button key={id} type="button" role="tab" aria-selected={tab === id} className="a-tab" onClick={() => setTab(id)}>
          {t(`tasks_tab_${id}` as const)} <em>{tabCounts[id]}</em>
        </button>
      ))}
    </div>
  )

  const chips =
    tab === 'all' ? (
      <div className="a-chips" role="group" aria-label={t('tasks_tab_all')}>
        {CHIPS.map((id) => (
          <button key={id} type="button" className="a-chip" aria-pressed={chip === id} onClick={() => setChip(id)}>
            {chipLabel(id)}
            {id === 'ready' && readyCount > 0 && <em>{readyCount}</em>}
          </button>
        ))}
      </div>
    ) : readyCount > 0 && !lite ? (
      <div className="a-chips">
        <button type="button" className="a-chip" onClick={() => { setTab('all'); setChip('ready') }}>
          {t('tasks_ready_chip', { n: readyCount })}
        </button>
      </div>
    ) : null

  const emptyText =
    tasks.length === 0
      ? t('tasks_none_yet_body')
      : tab === 'today' && chip === 'active'
        ? t('tasks_empty_today')
        : tab === 'upcoming'
          ? t('tasks_empty_upcoming')
          : searchQuery.trim() || tagFilter !== 'all' || selection !== ALL_SELECTION
            ? t('tasks_none_match')
            : t(`tasks_empty_${chip === 'ready' ? 'review' : chip}` as const)

  const rows =
    isRefreshing && tasks.length === 0 ? (
      <div aria-busy="true" aria-label={t('tasks_getting')}>
        {[0, 1].map((n) => (
          <div key={n} className="a-trow">
            <div className="a-trow-main">
              <span className="a-skel t" />
              <span className="a-skel s" />
            </div>
          </div>
        ))}
        <p className="a-grp">{t('tasks_getting')}</p>
      </div>
    ) : filtered.length === 0 ? (
      <div className="a-pad">
        <h3 className="a-body-b">{tasks.length === 0 ? t('tasks_none_yet_title') : t('tasks_none_found')}</h3>
        <p className="a-body-sm a-muted">{emptyText}</p>
        {tasks.length === 0 && (
          <button type="button" onClick={() => setFormOpen(true)} className="a-btn compact">
            {t('tasks_new_task')}
          </button>
        )}
      </div>
    ) : (
      <TaskListView
        tasks={filtered}
        allById={taskById}
        initiativeById={initiativeById}
        onSelect={(task) => openTask(task.id)}
        sections={sections}
        grouping={tab === 'today' && statusTab === 'active' ? 'today' : 'list'}
        onToggleDone={markDone}
        onCheck={checkIt}
        selectedId={selectedTaskId}
        selection={
          picking.active
            ? { ticked: picking.selectedIds, onToggle: afterPick(picking.toggle), onToggleMany: afterPick(picking.setMany) }
            : undefined
        }
      />
    )

  const noticeNode = notice && (
    <div className="a-stale" role="status">
      <AlertTriangle className="a-ic" aria-hidden="true" />
      <span>{notice}</span>
      <button type="button" onClick={() => setNotice(null)} className="a-ib" aria-label={t('close')}>
        <X className="a-ic sm" aria-hidden="true" />
      </button>
    </div>
  )

  const briefing = selection.initiativeId && (
    <div className="a-pad">
      {briefingFor === selection.initiativeId ? (
        <InitiativeBriefing
          initiativeId={selection.initiativeId}
          name={initiativeById.get(selection.initiativeId)?.name ?? t('tasks_initiative')}
          onClose={() => setBriefingFor(null)}
        />
      ) : (
        <button type="button" onClick={() => setBriefingFor(selection.initiativeId ?? null)} className="a-btn compact secondary">
          <ClipboardList className="a-ic sm" aria-hidden="true" />
          {t('tasks_briefing')}
        </button>
      )}
    </div>
  )

  const staleNote = unreachable && (
    <div className="a-stale" role="status">
      <AlertTriangle className="a-ic" aria-hidden="true" />
      <span>{t('tasks_unreachable')}</span>
    </div>
  )

  const listPane = (
    <>
      <PaneHead title={t('place_tasks')}>{headButtons}</PaneHead>
      <p className="a-cap a-muted tk-sub">{t('fact_active', { n: activeCount })}</p>
      {tabs}
      {chips}
      {controls}
      <div className="tk-scroll">
        {staleNote}
        {noticeNode}
        {briefing}
        {!lite && <WorkingNowPanel sessions={liveSessions} taskById={taskById} onSelect={openTask} />}
        {rows}
        <AwayLinkRow />
      </div>
    </>
  )

  const viewHead = (
    <>
      <PaneHead title={view === 'board' ? t('web_board') : t('web_timeline')}>{headButtons}</PaneHead>
      {controls}
      {staleNote}
      {noticeNode}
    </>
  )

  const emptyDetail = checkingOpen ? (
    <>
      <PaneHead title={t('tasks_how_checking')} back={<span />} />
      <div className="tk-body">
        <div className="a-pad tk-stack">
          <VerificationBar onVerifyNow={() => setSweepOpen(true)} onAudit={() => setAuditOpen(true)} />
        </div>
      </div>
    </>
  ) : (
    <div className="a-empty">
      <p className="a-body-sm a-muted">{t('tasks_pick_one')}</p>
      {!lite && (
        <button type="button" className="a-btn compact secondary" onClick={() => setCheckingOpen(true)}>
          {t('tasks_how_checking')}
        </button>
      )}
    </div>
  )

  const viewBody =
    view === 'board' ? (
      <TaskBoardView tasks={filtered} allById={taskById} onSelect={(task) => openTask(task.id)} onMove={handleStatusMove} />
    ) : (
      <TaskGanttView
        tasks={filtered}
        allById={taskById}
        initiativeById={initiativeById}
        onSelect={(task) => openTask(task.id)}
        onReschedule={handleReschedule}
        sessions={recentWork.data ?? undefined}
      />
    )

  let detail
  if (away) {
    detail = (
      <>
        <PaneHead
          title={t('tasks_away_title')}
          back={
            <button type="button" className="a-ib a-back" onClick={closeDetail} aria-label={t('tasks_back')}>
              <ChevronDown className="a-ic" aria-hidden="true" style={{ transform: 'rotate(90deg)' }} />
            </button>
          }
        />
        <div className="tk-body">
          <div className="a-pad">
            <AwayPage owner onOpenTask={openTask} onAudit={() => setAuditOpen(true)} />
          </div>
        </div>
      </>
    )
  } else if (selectedTask) {
    detail = (
      <TaskDetailMain
        task={selectedTask}
        allTasks={tasks}
        onClose={closeDetail}
        onDetails={() => setSideOpen(true)}
        onUpdate={handleUpdate}
        onDelete={handleDelete}
        onRestore={handleRestore}
        onSelectTask={openTask}
        onCreateSubtask={handleCreateSubtask}
        busy={busy}
      />
    )
  } else if (wideView) {
    detail = (
      <>
        {viewHead}
        <div className="tk-view">{isRefreshing && tasks.length === 0 ? rows : viewBody}</div>
      </>
    )
  } else {
    detail = emptyDetail
  }

  return (
    <WorkingNowContext.Provider value={workingMap}>
      <StaleIdsContext.Provider value={stale.data ?? NO_STALE}>
        <div className="tk-page">
          <Panes
            list={wideView ? undefined : listPane}
            detail={detail}
            side={selectedTask && !away ? <TaskDetailSide task={selectedTask} onClose={() => setSideOpen(false)} /> : undefined}
            showDetail={detailOpen || wideView}
            sideOpen={sideOpen}
            listLabel={t('place_tasks')}
            detailLabel={selectedTask?.title ?? t('place_tasks')}
            sideLabel={t('inbox_details')}
          />

          {!detailOpen && (
            <button type="button" className="a-fab" onClick={() => setFormOpen(true)}>
              <Plus className="a-ic" aria-hidden="true" />
              {t('tasks_new_task')}
            </button>
          )}

          {doneAsk && taskById.get(doneAsk.id) && (
            <Dialog
              kind="ask"
              title={t('tasks_done_one_title')}
              body={
                doneAsk.why === 'check'
                  ? t('tasks_done_check_body')
                  : tn('tasks_done_one_body', dependentsOf({ id: doneAsk.id }, taskById.values()).length, { title: taskById.get(doneAsk.id)?.title ?? '' })
              }
              keepLabel={doneAsk.why === 'check' ? t('tasks_done_check_keep') : t('tasks_done_one_keep')}
              actLabel={t('tasks_mark_done')}
              onKeep={() => setDoneAsk(null)}
              onAct={() => {
                const ask = doneAsk
                setDoneAsk(null)
                void applyUpdate(ask.id, ask.patch)
              }}
            />
          )}

          {bulkDonePending && (
            <Dialog
              kind="ask"
              title={t('bulk_done_title', { n: bulkDonePending.length })}
              body={t('bulk_done_body_undo', { n: bulkDonePending.length })}
              keepLabel={t('bulk_done_keep')}
              actLabel={t('bulk_done_act', { n: bulkDonePending.length })}
              onKeep={() => setBulkDonePending(null)}
              onAct={() => void confirmBulkDone()}
            />
          )}

          <SweepSheet
            key={sweepOpen ? 'open' : 'closed'}
            open={sweepOpen}
            onClose={() => setSweepOpen(false)}
            inputs={{ initiativeId: selection.initiativeId ?? null, selectedIds: picking.selected.map((t) => t.id) }}
            initiativeName={selection.initiativeId ? initiativeById.get(selection.initiativeId)?.name : undefined}
          />
          <AuditSheet open={auditOpen} onClose={() => setAuditOpen(false)} />

          <OutboxGroup
            entries={taskOutbox.entries}
            state={taskOutbox.state}
            inHub={false}
            textOf={titleOf}
            onRemove={(id) => void taskOutbox.remove(id)}
            onSendNow={() => void taskOutbox.sendNow()}
          />

          {listsOpen && (
            <TasksSidebar
              spaces={spaces}
              folders={folders}
              initiatives={initiatives}
              tasks={tasks}
              selection={selection}
              onSelect={setSelection}
              onTaxonomyChanged={loadTaxonomy}
              onClose={() => setListsOpen(false)}
            />
          )}

          <TaskFormModal
            open={formOpen}
            onClose={() => setFormOpen(false)}
            spaces={spaces}
            folders={folders}
            initiatives={initiatives}
            tasks={tasks}
            onTaxonomyChanged={loadTaxonomy}
            onCreate={handleCreate}
          />
        </div>
      </StaleIdsContext.Provider>
    </WorkingNowContext.Provider>
  )
}

function chipLabel(id: TaskChip): string {
  const status = chipStatus(id)
  if (status) return STATUS_LABELS[status]
  return id === 'archived' ? t('tasks_chip_archived') : t('tasks_chip_active')
}
