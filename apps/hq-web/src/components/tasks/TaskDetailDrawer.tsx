import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { X, Trash2, Pencil, Check, X as XIcon, CornerLeftUp } from 'lucide-react'
import type { TaskItem, TaskStatus } from '~/lib/tasksApi'
import { STATUS_LABELS, STATUS_ORDER } from '~/lib/tasksApi'
import { TaskRelations } from './TaskRelations'
import { TaskComments } from './TaskComments'
import { TaskSessions } from './TaskSessions'
import { TaskTime } from './TaskTime'
import { DateRangeInputs, EstimateInput, PRIORITIES, SectionLabel, isInvalidEstimate, parseEstimate } from './taskFields'
import { formatLocalTime } from './timeFormat'
import { MarkdownViewer } from '../MarkdownViewer'

interface Props {
  task: TaskItem | null
  allTasks: TaskItem[]
  onClose: () => void
  onUpdate: (id: string, patch: Record<string, unknown>) => Promise<void>
  onDelete: (id: string, cascade: boolean) => Promise<void>
  onSelectTask: (id: string) => void
  onCreateSubtask: (parent: TaskItem, title: string) => Promise<void>
  busy: boolean
}

const DATE_INPUT_CLASS =
  'px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400'

export function TaskDetailDrawer({
  task,
  allTasks,
  onClose,
  onUpdate,
  onDelete,
  onSelectTask,
  onCreateSubtask,
  busy,
}: Props) {
  const [editing, setEditing] = useState(false)
  const [draftTitle, setDraftTitle] = useState('')
  const [draftDescription, setDraftDescription] = useState('')
  const [draftTags, setDraftTags] = useState('')
  const [draftDueDate, setDraftDueDate] = useState('')
  const [draftStartDate, setDraftStartDate] = useState('')
  const [draftEstimate, setDraftEstimate] = useState('')
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  useEffect(() => {
    if (!task) return
    setEditing(false)
    setDraftTitle(task.title)
    setDraftDescription(task.description)
    setDraftTags(task.tags.join(', '))
    setDraftDueDate(task.due_date ?? '')
    setDraftStartDate(task.start_date ?? '')
    setDraftEstimate(task.estimate_minutes === null ? '' : String(task.estimate_minutes))
    setConfirmingDelete(false)
  }, [task])

  if (!task) return null

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
      due_date: draftDueDate || null,
      start_date: draftStartDate || null,
      estimate_minutes: parseEstimate(draftEstimate),
    })
    setEditing(false)
  }

  const parent = task.parent_task_id ? allTasks.find((t) => t.id === task.parent_task_id) : undefined

  const handleDeleteClick = () => {
    if (task.subtask_count > 0) setConfirmingDelete(true)
    else onDelete(task.id, false)
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
              <span className="text-[11px] font-bold text-neutral-500">{task.display_id}</span>
              {task.priority && (
                <span className="text-[11px] font-semibold uppercase text-amber-400">
                  {task.priority}
                </span>
              )}
            </div>
            {editing ? (
              <input
                value={draftTitle}
                onChange={(e) => setDraftTitle(e.target.value)}
                className="text-base font-semibold text-white bg-transparent border-b border-white/20 focus:outline-none focus:border-emerald-400 mt-1 w-full"
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
                  className="p-1.5 rounded-lg text-emerald-400 hover:bg-white/10 transition-colors disabled:opacity-50"
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
            ) : (
              <button
                type="button"
                onClick={() => setEditing(true)}
                className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
                title="Edit"
              >
                <Pencil className="w-4 h-4" />
              </button>
            )}
            <button
              type="button"
              onClick={handleDeleteClick}
              disabled={busy}
              className="p-1.5 rounded-lg text-neutral-400 hover:text-rose-400 hover:bg-white/10 transition-colors disabled:opacity-50"
              title="Delete"
            >
              <Trash2 className="w-4 h-4" />
            </button>
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
          <div className="flex items-center justify-between gap-3 px-6 py-3 border-b border-rose-500/20 bg-rose-500/10">
            <span className="text-xs text-rose-300">
              Delete {task.display_id} and its {task.subtask_count} sub-task{task.subtask_count === 1 ? '' : 's'}?
            </span>
            <div className="flex items-center gap-2 shrink-0">
              <button
                type="button"
                onClick={() => onDelete(task.id, true)}
                disabled={busy}
                className="px-3 py-1 rounded-lg text-xs font-semibold text-rose-300 border border-rose-500/40 hover:bg-rose-500/20 disabled:opacity-50"
              >
                Delete all
              </button>
              <button
                type="button"
                onClick={() => setConfirmingDelete(false)}
                className="px-3 py-1 rounded-lg text-xs text-neutral-400 hover:text-neutral-200"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {parent && (
          <button
            type="button"
            onClick={() => onSelectTask(parent.id)}
            className="flex items-center gap-1.5 px-6 py-2 border-b border-white/10 text-left text-[11px] text-neutral-400 hover:text-white"
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
              disabled={busy || status === task.status}
              onClick={() => onUpdate(task.id, { status, expected_status: task.status })}
              className={`px-3 py-1.5 rounded-xl text-xs font-semibold transition-all disabled:cursor-default ${
                status === task.status
                  ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/40'
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
                className="w-full px-3.5 py-2.5 rounded-xl text-sm text-neutral-200 border bg-black/30 focus:outline-none focus:ring-1 focus:ring-emerald-400 resize-none"
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
                  className={`px-3 py-1 rounded-lg text-[11px] font-semibold uppercase transition-all ${
                    p === task.priority
                      ? 'bg-amber-500/20 text-amber-400 border border-amber-500/40'
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
              <div>
                <SectionLabel>Tags (comma-separated; a routing tag like "hq" notifies that agent)</SectionLabel>
                <input
                  value={draftTags}
                  onChange={(e) => setDraftTags(e.target.value)}
                  placeholder="hq, backend, infra"
                  className="w-full px-3 py-2 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400"
                  style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                />
              </div>
            </>
          )}

          {!editing && (task.tags.length > 0 || task.due_date || task.start_date || task.work_started_at || task.first_ready_for_review_at || task.completed_at) && (
            <div className="flex items-center gap-2 flex-wrap text-xs ">
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

          <TaskTime key={`time-${task.id}`} task={task} />
          <TaskSessions key={task.id} taskId={task.id} />

          <TaskComments key={task.id} taskId={task.id} />
        </div>
      </div>
    </div>
  )

  return typeof document !== 'undefined' ? createPortal(content, document.body) : content
}
