import { hqFetch, readJson } from './hqAuth'

export type TaskStatus = 'to_do' | 'in_progress' | 'blocked' | 'ready_for_review' | 'complete'
export type TaskPriority = 'urgent' | 'high' | 'normal' | 'low'

export interface Space {
  id: string
  name: string
  slug: string
  created_at?: string
}

export interface Folder {
  id: string
  space_id: string
  name: string
  slug: string
}

export interface Initiative {
  id: string
  space_id: string
  folder_id: string | null
  name: string
  slug: string
  id_prefix: string
}

export interface TaskItem {
  id: string
  display_id: string
  initiative_id: string
  title: string
  description: string
  status: TaskStatus
  priority: TaskPriority | null
  due_date: string | null
  start_date: string | null
  /** UTC 'YYYY-MM-DD HH:MM:SS' of the first move into in_progress; null = unknown. */
  work_started_at: string | null
  /** UTC timestamp of the first move into ready_for_review; null = unknown. */
  first_ready_for_review_at: string | null
  /** UTC timestamp of the latest move into complete; null = not complete or not recorded. */
  completed_at: string | null
  /** Set on sub-tasks; nesting is one level deep. */
  parent_task_id: string | null
  tags: string[]
  /** Internal ids of the tasks this one waits for. */
  depends_on: string[]
  /** Display ids of dependencies that are not complete yet. */
  blocked_by: string[]
  subtask_count: number
  subtask_done: number
  created_by: string
  created_at: string
  updated_at: string
}

/** Update responses carry soft-dependency warnings alongside the task. */
export type TaskUpdateResult = TaskItem & { warnings?: string[] }

export interface TaskComment {
  id: number
  author: string
  body: string
  created_at: string
}

interface ListTasksFilter {
  space_id?: string
  initiative_id?: string
  status?: TaskStatus
  tag?: string
  priority?: TaskPriority
}

export const HTTP_CONFLICT = 409

/** Rows per request. The server caps a page at 500, so a longer list takes several. */
export const TASK_PAGE_SIZE = 500

interface TaskPage {
  tasks: TaskItem[]
  total: number
  has_more: boolean
}

/** Follows `has_more` so a list longer than one page is never cut. An empty page ends the loop. */
export async function collectTaskPages(
  fetchPage: (offset: number) => Promise<TaskPage>
): Promise<{ count: number; total: number; tasks: TaskItem[] }> {
  // A task edited between two requests moves to the front of the list and shifts
  // the later pages by one, so a row can arrive twice: the id keeps it once.
  const byId = new Map<string, TaskItem>()
  let total = 0
  let read = 0
  for (;;) {
    const page = await fetchPage(read)
    read += page.tasks.length
    for (const task of page.tasks) byId.set(task.id, task)
    total = page.total
    if (!page.has_more || page.tasks.length === 0) break
  }
  return { count: byId.size, total, tasks: [...byId.values()] }
}

export async function fetchTasksClient(
  filter?: ListTasksFilter
): Promise<{ count: number; total: number; tasks: TaskItem[] }> {
  const qs = new URLSearchParams()
  if (filter?.space_id) qs.set('space_id', filter.space_id)
  if (filter?.initiative_id) qs.set('initiative_id', filter.initiative_id)
  if (filter?.status) qs.set('status', filter.status)
  if (filter?.tag) qs.set('tag', filter.tag)
  if (filter?.priority) qs.set('priority', filter.priority)
  qs.set('limit', String(TASK_PAGE_SIZE))
  return collectTaskPages(async (offset) => {
    qs.set('offset', String(offset))
    return readJson(await hqFetch(`/api/tasks?${qs.toString()}`))
  })
}

export async function fetchSpacesClient(): Promise<{ spaces: Space[] }> {
  return readJson(await hqFetch('/api/spaces'))
}

export async function createSpaceClient(name: string): Promise<Space> {
  return readJson(
    await hqFetch('/api/spaces', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name }),
    })
  )
}

export async function fetchInitiativesClient(spaceId?: string): Promise<{ initiatives: Initiative[] }> {
  const qs = spaceId ? `?space_id=${encodeURIComponent(spaceId)}` : ''
  return readJson(await hqFetch(`/api/initiatives${qs}`))
}

export async function fetchFoldersClient(spaceId?: string): Promise<{ folders: Folder[] }> {
  const qs = spaceId ? `?space_id=${encodeURIComponent(spaceId)}` : ''
  return readJson(await hqFetch(`/api/folders${qs}`))
}

export async function createFolderClient(spaceId: string, name: string): Promise<Folder> {
  return readJson(
    await hqFetch('/api/folders', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ space_id: spaceId, name }),
    })
  )
}

export async function createInitiativeClient(spaceId: string, name: string, folder?: string): Promise<Initiative> {
  return readJson(
    await hqFetch('/api/initiatives', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ space_id: spaceId, name, folder }),
    })
  )
}

interface CreateTaskInput {
  title: string
  description?: string
  initiative_id?: string
  space_id?: string
  folder?: string
  initiative?: string
  priority?: TaskPriority
  due_date?: string
  start_date?: string
  parent_task_id?: string
  depends_on?: string[]
  tags?: string[]
  created_by?: string
}

export async function createTaskClient(input: CreateTaskInput): Promise<TaskItem> {
  return readJson(
    await hqFetch('/api/tasks', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    })
  )
}

export interface UpdateTaskInput {
  title?: string
  description?: string
  status?: TaskStatus
  priority?: TaskPriority | null
  due_date?: string | null
  start_date?: string | null
  parent_task_id?: string | null
  add_depends_on?: string[]
  remove_depends_on?: string[]
  tags?: string[]
  expected_status?: TaskStatus
}

export async function updateTaskClient(id: string, input: UpdateTaskInput): Promise<TaskUpdateResult> {
  return readJson(
    await hqFetch(`/api/tasks/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    })
  )
}

export async function deleteTaskClient(
  id: string,
  cascade = false
): Promise<{ deleted: boolean; id: string; deleted_ids: string[] }> {
  const qs = cascade ? '?cascade=true' : ''
  return readJson(await hqFetch(`/api/tasks/${encodeURIComponent(id)}${qs}`, { method: 'DELETE' }))
}

export async function fetchCommentsClient(taskId: string): Promise<{ comments: TaskComment[] }> {
  return readJson(await hqFetch(`/api/tasks/${encodeURIComponent(taskId)}/comments`))
}

export async function addCommentClient(taskId: string, body: string, author = 'operator'): Promise<TaskComment> {
  return readJson(
    await hqFetch(`/api/tasks/${encodeURIComponent(taskId)}/comments`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ body, author }),
    })
  )
}

export const STATUS_LABELS: Record<TaskStatus, string> = {
  to_do: 'To Do',
  in_progress: 'In Progress',
  blocked: 'Blocked',
  ready_for_review: 'Ready for Review',
  complete: 'Complete',
}

export const STATUS_ORDER: TaskStatus[] = ['to_do', 'in_progress', 'blocked', 'ready_for_review', 'complete']

/**
 * Rust's `datetime('now')` (used for created_at/updated_at across hq-db,
 * this table included) writes naive UTC as "YYYY-MM-DD HH:MM:SS" with no
 * timezone marker. `new Date(...)` on that string parses it as local time
 * in most browsers, silently shifting every timestamp by the viewer's UTC
 * offset. Normalize to a real UTC ISO string before handing it to Date.
 */
export function parseSqliteUtc(s: string): Date {
  const iso = s.includes('T') ? s : s.replace(' ', 'T')
  return new Date(iso.endsWith('Z') ? iso : `${iso}Z`)
}
