import { HqHttpError } from '~/lib/hqAuth'
import { createFileRoute } from '@tanstack/react-router'
import { useState, useMemo, useEffect, useRef } from 'react'
import { CheckSquare, Plus, RefreshCw, Search, Loader2, PanelLeftOpen, List, Columns3, GanttChart, X } from 'lucide-react'
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
  type WorkSession,
  type TaskStatus,
  type TaskUpdateResult,
  type UpdateTaskInput,
} from '~/lib/tasksApi'
import { TaskListView } from '~/components/tasks/TaskListView'
import { TaskBoardView } from '~/components/tasks/TaskBoardView'
import { TaskGanttView } from '~/components/tasks/TaskGanttView'
import { TaskDetailDrawer } from '~/components/tasks/TaskDetailDrawer'
import { TaskFormModal } from '~/components/tasks/TaskFormModal'
import { ALL_SELECTION, TasksSidebar, type TaskSelection } from '~/components/tasks/TasksSidebar'
import { useTasksData } from '~/components/tasks/useTasksData'
import { usePolled } from '~/components/sessions/usePolled'
import { StaleIdsContext, stableSet } from '~/components/tasks/staleContext'
import { WorkingNowContext, workingNow } from '~/components/tasks/workingContext'
import { WorkingNowPanel } from '~/components/tasks/WorkingNowPanel'
import { VaultNoteDrawer } from '~/components/VaultNoteDrawer'
import { useRefreshOn } from '~/lib/useRefreshOn'

type TasksView = 'list' | 'board' | 'timeline'

const VIEWS: { id: TasksView; label: string; icon: typeof List }[] = [
  { id: 'list', label: 'List', icon: List },
  { id: 'board', label: 'Board', icon: Columns3 },
  { id: 'timeline', label: 'Timeline', icon: GanttChart },
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

const errorMessage = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback)

interface TaskFilters {
  statusTab: StatusTab
  statusFilter: TaskStatus | 'all'
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
      // The board shows every status as a column, so the coarse tab is ignored there.
      if (f.statusTab === 'active' && t.status === 'complete') return false
      if (f.statusTab === 'complete' && t.status !== 'complete') return false
    }
    if (f.tagFilter !== 'all' && !t.tags.includes(f.tagFilter)) return false
    if (!matchesSelection(t, f.selection, initiativeById)) return false
    return !f.searchQuery.trim() || matchesSearch(t, f.searchQuery)
  })
}

function TasksPage() {
  const view: TasksView = Route.useSearch().view ?? 'list'
  const navigate = Route.useNavigate()
  const [notice, setNotice] = useState<string | null>(null)
  const [statusTab, setStatusTab] = useState<StatusTab>('active')
  const [statusFilter, setStatusFilter] = useState<TaskStatus | 'all'>('all')
  const [tagFilter, setTagFilter] = useState<string>('all')
  const [selection, setSelection] = useState<TaskSelection>(ALL_SELECTION)
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [formOpen, setFormOpen] = useState(false)
  const [busy, setBusy] = useState(false)

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
    view === 'timeline'
  )

  const live = usePolled(
    'live-work',
    async () => (await fetchRecentWorkSessionsClient(LIVE_WORK_DAYS)).work_sessions.filter((s) => s.ended_at === null),
    LIVE_WORK_POLL_MS
  )

  useRefreshOn(['task:sync'], live.refresh)

  const archived = usePolled(
    'archived-tasks',
    async () => (await fetchTasksClient({ archived: true })).tasks,
    ARCHIVED_POLL_MS,
    statusTab === 'archived'
  )
  const staleSeen = useRef<ReadonlySet<string>>(NO_STALE)
  const stale = usePolled(
    'stale-task-ids',
    async () => {
      const next = new Set((await fetchStaleTasksClient()).tasks.map((s) => s.task_id))
      staleSeen.current = stableSet(next, staleSeen.current)
      return staleSeen.current
    },
    STALE_POLL_MS
  )
  useRefreshOn(['task:sync'], stale.refresh)
  useRefreshOn(['task:sync'], recentWork.refresh)
  const archivedTasks = archived.data ?? []

  const { task: linkedTaskId } = Route.useSearch()
  useEffect(() => {
    if (linkedTaskId) setSelectedTaskId(linkedTaskId)
  }, [linkedTaskId, setSelectedTaskId])

  const selectedTask = useMemo(
    () => tasks.find((t) => t.id === selectedTaskId) ?? archivedTasks.find((t) => t.id === selectedTaskId) ?? null,
    [tasks, archivedTasks, selectedTaskId]
  )

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
        { statusTab, statusFilter, tagFilter, selection, searchQuery, view },
        initiativeById
      ),
    [tasks, archivedTasks, statusTab, statusFilter, tagFilter, selection, initiativeById, searchQuery, view]
  )

  const setView = (next: TasksView) =>
    navigate({ search: next === 'list' ? {} : { view: next }, replace: true })

  const showWarnings = (result: TaskUpdateResult) => {
    if (result.warnings?.length) setNotice(result.warnings.join(' '))
  }

  const handleCreate = async (input: Parameters<typeof createTaskClient>[0]) => {
    const task = await createTaskClient(input)
    upsertTask(task)
    await loadTaxonomy()
  }

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

  const handleUpdate = (id: string, patch: Record<string, unknown>) =>
    withBusy(async () => {
      const task = await updateTaskClient(id, patch)
      upsertTask(task)
      showWarnings(task)
    }, 'Update failed')

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
      setNotice(conflict ? conflictMessage : errorMessage(e, 'Update failed'))
      if (conflict) loadTasks()
    }
  }

  const handleStatusMove = (task: TaskItem, status: TaskStatus) =>
    applyOptimistic(
      task,
      { status, expected_status: task.status },
      `${task.display_id} was moved by someone else in the meantime, so the board has been refreshed.`
    )

  const handleReschedule = (task: TaskItem, patch: UpdateTaskInput) =>
    applyOptimistic(task, patch, `${task.display_id} changed in the meantime, so the timeline has been refreshed.`)

  const handleCreateSubtask = async (parent: TaskItem, title: string) => {
    try {
      const task = await createTaskClient({ title, parent_task_id: parent.id, created_by: 'operator' })
      upsertTask(task)
    } catch (e) {
      setNotice(errorMessage(e, 'Could not add the sub-task'))
    }
  }

  const handleDelete = (id: string, cascade: boolean) =>
    withBusy(async () => {
      const res = await deleteTaskClient(id, cascade)
      for (const deletedId of res.deleted_ids ?? [id]) removeTask(deletedId)
      setSelectedTaskId(null)
    }, 'Delete failed')

  const handleRestore = (id: string) =>
    withBusy(async () => {
      upsertTask(await restoreTaskClient(id))
      await archived.refresh()
    }, 'Restore failed')

  const activeCount = tasks.filter((t) => t.status !== 'complete').length

  return (
    <WorkingNowContext.Provider value={workingMap}>
    <StaleIdsContext.Provider value={stale.data ?? NO_STALE}>
    <div className="flex h-full min-h-0 w-full max-w-full overflow-x-hidden">
      <TasksSidebar
        spaces={spaces}
        folders={folders}
        initiatives={initiatives}
        tasks={tasks}
        selection={selection}
        onSelect={setSelection}
        onTaxonomyChanged={loadTaxonomy}
        mobileOpen={mobileSidebarOpen}
        onMobileClose={() => setMobileSidebarOpen(false)}
      />

      <div className="flex-1 flex flex-col h-full overflow-y-auto overflow-x-hidden px-4 py-6 sm:px-8 w-full max-w-full min-w-0 box-border">
        <TasksHeader
          activeCount={activeCount}
          isRefreshing={isRefreshing}
          onRefresh={loadTasks}
          onNew={() => setFormOpen(true)}
          onOpenSidebar={() => setMobileSidebarOpen(true)}
        />

        <TasksFilterBar
          view={view}
          setView={setView}
          statusTab={statusTab}
          setStatusTab={setStatusTab}
          statusFilter={statusFilter}
          setStatusFilter={setStatusFilter}
          tagFilter={tagFilter}
          setTagFilter={setTagFilter}
          allTags={allTags}
          searchQuery={searchQuery}
          setSearchQuery={setSearchQuery}
        />

        <WorkingNowPanel sessions={liveSessions} taskById={taskById} onSelect={setSelectedTaskId} />

        {notice && (
          <div className="mb-4 flex items-start justify-between gap-3 px-3.5 py-2.5 rounded-xl border border-amber-500/20 bg-amber-500/10 text-xs text-amber-300">
            <span>{notice}</span>
            <button type="button" onClick={() => setNotice(null)} className="shrink-0 text-amber-300/70 hover:text-amber-200">
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        )}

        {isRefreshing && tasks.length === 0 ? (
          <div className="py-16 flex items-center justify-center text-neutral-500">
            <Loader2 className="w-5 h-5 animate-spin" />
          </div>
        ) : filtered.length === 0 && view === 'list' ? (
          <div className="py-16 flex flex-col items-center justify-center text-center p-8 rounded-2xl border border-dashed border-white/10 bg-white/[0.01]">
            <div className="p-4 rounded-2xl bg-white/5 text-neutral-500 mb-3">
              <CheckSquare className="w-8 h-8" />
            </div>
            <h3 className="text-sm font-bold text-neutral-300">No tasks found</h3>
            <p className="text-xs text-neutral-500 max-w-sm mt-1">
              {statusTab === 'active' ? 'Nothing active right now.' : 'No tasks match the current filters.'}
            </p>
          </div>
        ) : view === 'board' ? (
          <TaskBoardView tasks={filtered} allById={taskById} onSelect={(t) => setSelectedTaskId(t.id)} onMove={handleStatusMove} />
        ) : view === 'timeline' ? (
          <TaskGanttView
            tasks={filtered}
            allById={taskById}
            initiativeById={initiativeById}
            onSelect={(t) => setSelectedTaskId(t.id)}
            onReschedule={handleReschedule}
            sessions={recentWork.data ?? undefined}
          />
        ) : (
          <TaskListView
            tasks={filtered}
            allById={taskById}
            initiativeById={initiativeById}
            onSelect={(t) => setSelectedTaskId(t.id)}
          />
        )}

        <TaskDetailDrawer
          task={selectedTask}
          allTasks={tasks}
          onClose={() => setSelectedTaskId(null)}
          onUpdate={handleUpdate}
          onDelete={handleDelete}
          onRestore={handleRestore}
          onSelectTask={setSelectedTaskId}
          onCreateSubtask={handleCreateSubtask}
          busy={busy}
        />

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

        <VaultNoteDrawer />
      </div>
    </div>
    </StaleIdsContext.Provider>
    </WorkingNowContext.Provider>
  )
}

interface TasksHeaderProps {
  activeCount: number
  isRefreshing: boolean
  onRefresh: () => void
  onNew: () => void
  onOpenSidebar: () => void
}

function TasksHeader({ activeCount, isRefreshing, onRefresh, onNew, onOpenSidebar }: TasksHeaderProps) {
  return (
    <div className="flex flex-row items-center justify-between gap-3 mb-4 sm:mb-6">
      <div className="flex items-center gap-2.5 min-w-0">
        <button
          type="button"
          onClick={onOpenSidebar}
          className="md:hidden p-2 rounded-xl bg-white/5 text-neutral-300 hover:text-white"
        >
          <PanelLeftOpen className="w-4 h-4" />
        </button>
        <div className="hidden sm:block p-2.5 rounded-xl hq-glass-card text-emerald-400">
          <CheckSquare className="w-5 h-5" />
        </div>
        <div>
          <h1 className="text-2xl font-semibold text-white tracking-tight" style={{ fontFamily: 'var(--font-heading)' }}>Tasks</h1>
          <p className="text-xs text-neutral-400 mt-0.5">{activeCount} active</p>
        </div>
      </div>

      <div className="flex items-center gap-2 shrink-0">
        <button
          type="button"
          onClick={onRefresh}
          aria-label="Refresh tasks"
          disabled={isRefreshing}
          className="hq-btn-ghost disabled:opacity-50"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isRefreshing ? 'animate-spin' : ''}`} />
          <span className="hidden sm:inline">Refresh</span>
        </button>
        <button
          type="button"
          onClick={onNew}
          className="hq-btn-primary !h-9 !px-4"
        >
          <Plus className="w-3.5 h-3.5" />
          <span className="hidden min-[420px]:inline">New Task</span>
          <span className="min-[420px]:hidden sr-only">New Task</span>
        </button>
      </div>
    </div>
  )
}

interface TasksFilterBarProps {
  view: TasksView
  setView: (v: TasksView) => void
  statusTab: StatusTab
  setStatusTab: (t: StatusTab) => void
  statusFilter: TaskStatus | 'all'
  setStatusFilter: (s: TaskStatus | 'all') => void
  tagFilter: string
  setTagFilter: (t: string) => void
  allTags: string[]
  searchQuery: string
  setSearchQuery: (q: string) => void
}

function TasksFilterBar(p: TasksFilterBarProps) {
  const { view, setView, statusTab, setStatusTab, statusFilter, setStatusFilter } = p
  const { tagFilter, setTagFilter, allTags, searchQuery, setSearchQuery } = p
  return (
    <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 sm:gap-3 mb-4 sm:mb-5 w-full max-w-full">
      <div className="flex items-center gap-2 flex-wrap w-full sm:w-auto min-w-0">
        <div className="flex items-center gap-1 p-1 rounded-xl hq-field self-start shrink-0">
          {VIEWS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => setView(id)}
              title={label}
              className={`px-2.5 py-1.5 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all ${
                view === id ? 'hq-seg-on' : 'text-neutral-400 hover:text-neutral-200'
              }`}
            >
              <Icon className="w-3.5 h-3.5 shrink-0" />
              <span className="hidden sm:inline">{label}</span>
            </button>
          ))}
        </div>

        {view !== 'board' && (
          <div className="flex items-center gap-1 p-1 rounded-xl hq-field self-start max-w-full overflow-x-auto no-scrollbar shrink-0">
            {(['active', 'all', 'complete', 'archived'] as StatusTab[]).map((tab) => (
              <button
                key={tab}
                type="button"
                onClick={() => setStatusTab(tab)}
                className={`px-3 py-1.5 rounded-lg text-xs font-semibold capitalize transition-all shrink-0 ${
                  statusTab === tab ? 'hq-seg-on' : 'text-neutral-400 hover:text-neutral-200'
                }`}
              >
                {tab}
              </button>
            ))}
          </div>
        )}

        <div className="flex items-center gap-2 w-full sm:w-auto min-w-0">
          <select
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value as TaskStatus | 'all')}
            title="Filter by status"
            className="px-2.5 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400 max-w-full truncate flex-1 min-w-0 sm:flex-none"
          >
            <option value="all">All statuses</option>
            {STATUS_ORDER.map((status) => (
              <option key={status} value={status}>{STATUS_LABELS[status]}</option>
            ))}
          </select>

          <select
            value={tagFilter}
            onChange={(e) => setTagFilter(e.target.value)}
            title="Filter by tag"
            className="px-2.5 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400 max-w-full truncate flex-1 min-w-0 sm:flex-none"
          >
            <option value="all">All tags</option>
            {allTags.map((tag) => (
              <option key={tag} value={tag}>{tag}</option>
            ))}
          </select>
        </div>
      </div>

      <div className="relative w-full sm:w-auto sm:max-w-xs sm:flex-1 min-w-0">
        <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-neutral-500" />
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search tasks..."
          className="w-full box-border pl-8 pr-3 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400 placeholder:text-neutral-600 transition-all"
        />
      </div>
    </div>
  )
}
