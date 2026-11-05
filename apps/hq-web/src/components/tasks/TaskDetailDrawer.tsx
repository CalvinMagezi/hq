import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { X, Archive, Pencil, Check, X as XIcon, CornerLeftUp } from 'lucide-react'
import type { TaskItem, TaskStatus } from '~/lib/tasksApi'
import { STATUS_LABELS, STATUS_ORDER } from '~/lib/tasksApi'
import { TaskRelations } from './TaskRelations'
import { TaskComments } from './TaskComments'
import { TaskSessions } from './TaskSessions'
import { TaskTime } from './TaskTime'
import { TaskLinks } from './TaskLinks'
import { TaskCheckpoint } from './TaskCheckpoint'
import { TaskActivity } from './TaskActivity'
import { TaskDecisions } from './TaskDecisions'
import { TaskVerification } from './verification/TaskVerification'
import { useIsStale } from './staleContext'
import { DateRangeInputs, EstimateInput, PRIORITIES, SectionLabel, isInvalidEstimate, parseEstimate } from './taskFields'
import { formatLocalTime } from './timeFormat'
import { Dialog } from '../ui/Dialog'
import { t } from '~/strings'
import { MarkdownViewer } from '../MarkdownViewer'

interface Props {
  task: TaskItem | null
  allTasks: TaskItem[]
  onClose: () => void
  onUpdate: (id: string, patch: Record<string, unknown>) => Promise<void>
  /** Brings an archived task back. */
  onRestore?: (id: string) => Promise<void>
  /** Archives the task; `purge` deletes an archived one for good. */
  onDelete: (id: string, cascade: boolean, purge?: boolean) => Promise<void>
  onSelectTask: (id: string) => void
  onCreateSubtask: (parent: TaskItem, title: string) => Promise<void>
  busy: boolean
}

const DATE_INPUT_CLASS =
  'px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-white'

export function TaskDetailDrawer({
  task,
  allTasks,
  onClose,
  onUpdate,
  onRestore,
  onDelete,
  onSelectTask,
  onCreateSubtask,
  busy,
}: Props) {
  const [editing, setEditing] = useState(false)
  const [draftTitle, setDraftTitle] = useState('')
  const [draftDescription, setDraftDescription] = useState('')
  const [draftTags, setDraftTags] = useState('')
  const [draftAssignees, setDraftAssignees] = useState('')
  const [draftDueDate, setDraftDueDate] = useState('')
  const [draftStartDate, setDraftStartDate] = useState('')
  const [draftEstimate, setDraftEstimate] = useState('')
  const [draftReason, setDraftReason] = useState('')
  const [draftWaiting, setDraftWaiting] = useState('')
  const [draftLong, setDraftLong] = useState(false)
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  useEffect(() => {
    if (!task) return
    setEditing(false)
    setDraftTitle(task.title)
    setDraftDescription(task.description)
    setDraftTags(task.tags.join(', '))
    setDraftAssignees(task.assignees.join(', '))
    setDraftDueDate(task.due_date ?? '')
    setDraftStartDate(task.start_date ?? '')
    setDraftEstimate(task.estimate_minutes === null ? '' : String(task.estimate_minutes))
    setDraftReason(task.blocked_reason ?? '')
    setDraftWaiting(task.waiting_on ?? '')
    setDraftLong(task.long_horizon)
    setConfirmingDelete(false)
  }, [task])

  const stale = useIsStale(task?.id ?? '')

  if (!task) return null
  const archived = task.archived_at !== null

  const handleSaveEdit = async () => {
    if (isInvalidEstimate(draftEstimate)) return
    const tags = draftTags
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean)
    await onUpdate(task.id, {
      title: draftTitle,
      description: draftDescription,
      tags,
      assignees: draftAssignees
        .split(',')
        .map((a) => a.trim())
        .filter(Boolean),
      due_date: draftDueDate || null,
      start_date: draftStartDate || null,
      estimate_minutes: parseEstimate(draftEstimate),
      long_horizon: draftLong,
      ...(task.status === 'blocked'
        ? { blocked_reason: draftReason.trim() || null, waiting_on: draftWaiting.trim() || null }
        : {}),
    })
    setEditing(false)
  }

  const parent = task.parent_task_id ? allTasks.find((t) => t.id === task.parent_task_id) : undefined

  const handleDeleteClick = () => {
    void onDelete(task.id, task.subtask_count > 0)
  }

  const content = (
    <div className="fixed inset-0 z-50 flex justify-end hq-scrim transition-all duration-300">
      <div className="flex-1" onClick={onClose} />

      <div
        className="w-full max-w-2xl h-full flex flex-col hq-modal border-l shadow-2xl overflow-hidden animate-in slide-in-from-right pad-safe-top pb-[var(--safe-bottom)]"
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-6 py-4 border-b flex-shrink-0"
          style={{ borderColor: 'rgba(255,255,255,0.08)' }}
        >
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-xs font-bold text-neutral-500">{task.display_id}</span>
              {task.priority && (
                <span className="text-xs font-semibold uppercase text-[var(--warn)]">
                  {task.priority}
                </span>
              )}
              {task.long_horizon && <span className="text-xs text-neutral-400">long running</span>}
              {stale && <span className="text-xs text-neutral-400" title="Nobody holds it and nothing has changed for a while">stale</span>}
            </div>
            {editing ? (
              <input
                value={draftTitle}
                onChange={(e) => setDraftTitle(e.target.value)}
                className="text-base font-semibold text-white bg-transparent border-b border-white/20 focus:outline-none focus:border-white mt-1 w-full"
              />
            ) : (
              <h2 className="text-base font-semibold text-white mt-1 break-words">{task.title}</h2>
            )}
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {editing ? (
              <>
                <button
                  type="button"
                  onClick={handleSaveEdit}
                  disabled={busy || isInvalidEstimate(draftEstimate)}
                  className="p-1.5 rounded-lg text-white hover:bg-white/10 transition-colors disabled:opacity-50"
                  title="Save"
                >
                  <Check className="w-4 h-4" />
                </button>
                <button
                  type="button"
                  onClick={() => setEditing(false)}
                  className="p-1.5 rounded-lg text-neutral-400 hover:bg-white/10 transition-colors"
                  title="Cancel"
                >
                  <XIcon className="w-4 h-4" />
                </button>
              </>
            ) : archived ? null : (
              <button
                type="button"
                onClick={() => setEditing(true)}
                className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
                title="Edit"
              >
                <Pencil className="w-4 h-4" />
              </button>
            )}
            {!archived && <button
              type="button"
              onClick={handleDeleteClick}
              disabled={busy}
              className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors disabled:opacity-50"
              title={t('chat_archive')}
            >
              <Archive className="w-4 h-4" />
            </button>}
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
            >
              <X className="w-5 h-5" />
            </button>
          </div>
        </div>

        {confirmingDelete && (
          <Dialog
            kind="red"
            title={t('delete_task_title')}
            body={t('delete_task_body', { title: task.title })}
            keepLabel={t('delete_task_keep')}
            actLabel={t('delete_task_act')}
            busyLabel={t('confirm_busy_deleting')}
            busy={busy}
            onKeep={() => setConfirmingDelete(false)}
            onAct={() => void onDelete(task.id, true, true)}
          />
        )}

        {archived && (
          <div className="flex items-center justify-between gap-3 px-6 py-3 border-b border-white/10 bg-white/[0.03]">
            <span className="text-xs text-neutral-300">
              Archived {formatLocalTime(task.archived_at ?? '')}. It keeps its comments, time and links.
            </span>
            {onRestore && (
              <button
                type="button"
                onClick={() => onRestore(task.id)}
                disabled={busy}
                className="px-3 py-1 rounded-lg text-xs font-semibold text-neutral-200 border border-white/20 hover:bg-white/10 disabled:opacity-50 shrink-0"
              >
                Restore
              </button>
            )}
            <button
              type="button"
              onClick={() => setConfirmingDelete(true)}
              disabled={busy}
              className="px-3 py-1 rounded-lg text-xs font-semibold border border-white/20 hover:bg-white/10 disabled:opacity-50 shrink-0"
              style={{ color: 'var(--accent-red)' }}
            >
              {t('delete_task_act')}
            </button>
          </div>
        )}

        {parent && (
          <button
            type="button"
            onClick={() => onSelectTask(parent.id)}
            className="flex items-center gap-1.5 px-6 py-2 border-b border-white/10 text-left text-xs text-neutral-400 hover:text-white"
          >
            <CornerLeftUp className="w-3 h-3 shrink-0" />
            Sub-task of <span className="text-neutral-500">{parent.display_id}</span>
            <span className="truncate">{parent.title}</span>
          </button>
        )}

        {/* Status pills */}
        <div className="flex items-center gap-1.5 flex-wrap px-6 py-3 border-b" style={{ borderColor: 'rgba(255,255,255,0.08)' }}>
          {STATUS_ORDER.map((status: TaskStatus) => (
            <button
              key={status}
              type="button"
              disabled={busy || archived || status === task.status}
              onClick={() => onUpdate(task.id, { status, expected_status: task.status })}
              className={`px-3 py-1.5 rounded-xl text-xs font-semibold transition-all disabled:cursor-default ${
                status === task.status
                  ? 'bg-transparent border hq-tone-ink'
                  : 'bg-white/5 text-neutral-400 border border-white/10 hover:text-white hover:border-white/20'
              }`}
            >
              {STATUS_LABELS[status]}
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-5 space-y-6">
          <div>
            <SectionLabel>Description</SectionLabel>
            {editing ? (
              <textarea
                value={draftDescription}
                onChange={(e) => setDraftDescription(e.target.value)}
                rows={5}
                className="w-full px-3.5 py-2.5 rounded-xl text-sm text-neutral-200 border bg-black/30 focus:outline-none focus:ring-1 focus:ring-white resize-none"
                style={{ borderColor: 'rgba(255,255,255,0.1)' }}
              />
            ) : task.description ? (
              <div className="text-sm text-neutral-200 leading-relaxed">
                <MarkdownViewer content={task.description} bare />
              </div>
            ) : (
              <p className="text-sm text-neutral-600 italic">No description</p>
            )}
          </div>

          <div>
            <SectionLabel className="mb-2">Priority</SectionLabel>
            <div className="flex items-center gap-1.5 flex-wrap">
              {PRIORITIES.map((p) => (
                <button
                  key={p}
                  type="button"
                  disabled={busy}
                  onClick={() => onUpdate(task.id, { priority: p === task.priority ? null : p })}
                  className={`px-3 py-1 rounded-lg text-xs font-semibold uppercase transition-all ${
                    p === task.priority
                      ? 'bg-transparent border hq-tone-warn'
                      : 'bg-white/5 text-neutral-500 border border-white/10 hover:text-neutral-300'
                  }`}
                >
                  {p}
                </button>
              ))}
            </div>
          </div>

          {editing && (
            <>
              <div className="flex items-end gap-3 flex-wrap">
                <DateRangeInputs
                  start={draftStartDate}
                  due={draftDueDate}
                  onStart={setDraftStartDate}
                  onDue={setDraftDueDate}
                  inputClass={DATE_INPUT_CLASS}
                  label={(text) => <SectionLabel>{text}</SectionLabel>}
                />
                <EstimateInput
                  value={draftEstimate}
                  onChange={setDraftEstimate}
                  inputClass={DATE_INPUT_CLASS}
                  label={(text) => <SectionLabel>{text}</SectionLabel>}
                />
              </div>
              {task.status === 'blocked' && (
                <div className="grid gap-3 sm:grid-cols-2">
                  <div>
                    <SectionLabel>Why it is blocked</SectionLabel>
                    <input
                      value={draftReason}
                      onChange={(e) => setDraftReason(e.target.value)}
                      maxLength={500}
                      placeholder="what is in the way"
                      className="w-full px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-white"
                      style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                    />
                  </div>
                  <div>
                    <SectionLabel>Waiting on</SectionLabel>
                    <input
                      value={draftWaiting}
                      onChange={(e) => setDraftWaiting(e.target.value)}
                      maxLength={500}
                      placeholder="a person, a task, an outside thing"
                      className="w-full px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-white"
                      style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                    />
                  </div>
                </div>
              )}
              <label className="flex items-center gap-2 text-xs text-neutral-300">
                <input type="checkbox" checked={draftLong} onChange={(e) => setDraftLong(e.target.checked)} />
                Long running: a session finishing a turn or exiting does not move this task
              </label>
              <div>
                <SectionLabel>Assigned to (comma-separated; an agent named here is notified)</SectionLabel>
                <input
                  value={draftAssignees}
                  onChange={(e) => setDraftAssignees(e.target.value)}
                  placeholder="hq, reviewer"
                  className="w-full px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-white"
                  style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                />
              </div>
              <div>
                <SectionLabel>Tags (comma-separated; what the task is about)</SectionLabel>
                <input
                  value={draftTags}
                  onChange={(e) => setDraftTags(e.target.value)}
                  placeholder="hq, backend, infra"
                  className="w-full px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-white"
                  style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                />
              </div>
            </>
          )}

          {!editing && task.status === 'blocked' && (task.blocked_reason || task.waiting_on) && (
            <div className="px-3 py-2 rounded-xl border hq-tone-warn bg-white/5 text-xs text-neutral-200 space-y-0.5">
              {task.blocked_reason && <p className="break-words">Blocked: {task.blocked_reason}</p>}
              {task.waiting_on && <p className="break-words text-neutral-400">Waiting on {task.waiting_on}</p>}
              {task.blocked_since && <p className="text-xs text-neutral-500">since {formatLocalTime(task.blocked_since)}</p>}
            </div>
          )}

          {!editing && (task.tags.length > 0 || task.assignees.length > 0 || task.due_date || task.start_date || task.work_started_at || task.first_ready_for_review_at || task.completed_at) && (
            <div className="flex items-center gap-2 flex-wrap text-xs ">
              {task.assignees.length > 0 && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-200 border border-white/10 break-all" title="Assigned to">
                  for {task.assignees.join(', ')}
                </span>
              )}
              {task.tags.map((tag) => (
                <span key={tag} className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-300 border border-white/10">
                  {tag}
                </span>
              ))}
              {task.start_date && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-400 border border-white/10">
                  Start {task.start_date}
                </span>
              )}
              {task.work_started_at && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-400 border border-white/10">
                  Work started {formatLocalTime(task.work_started_at)}
                </span>
              )}
              {task.first_ready_for_review_at && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-400 border border-white/10">
                  First ready for review {formatLocalTime(task.first_ready_for_review_at)}
                </span>
              )}
              {task.completed_at && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-400 border border-white/10">
                  Completed {formatLocalTime(task.completed_at)}
                </span>
              )}
              {task.due_date && (
                <span className="px-2.5 py-1 rounded-full bg-white/5 text-neutral-400 border border-white/10">
                  Due {task.due_date}
                </span>
              )}
            </div>
          )}

          <TaskRelations
            task={task}
            allTasks={allTasks}
            busy={busy}
            onUpdate={onUpdate}
            onSelectTask={onSelectTask}
            onCreateSubtask={onCreateSubtask}
          />

          <TaskDecisions key={`decisions-${task.id}`} taskId={task.id} />
          <TaskVerification key={`verification-${task.id}`} task={task} onUpdate={onUpdate} />
          <TaskCheckpoint key={`checkpoint-${task.id}`} task={task} />
          <TaskActivity key={`activity-${task.id}`} task={task} />
          <TaskLinks key={`links-${task.id}`} task={task} />
          <TaskTime key={`time-${task.id}`} task={task} />
          <TaskSessions key={`sessions-${task.id}`} taskId={task.id} />

          <TaskComments key={`comments-${task.id}`} taskId={task.id} />
        </div>
      </div>
    </div>
  )

  return typeof document !== 'undefined' ? createPortal(content, document.body) : content
}
