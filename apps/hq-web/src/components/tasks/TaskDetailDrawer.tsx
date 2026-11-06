import { useEffect, useState } from 'react'
import { ArrowLeft, Info, MoreVertical, Pencil, Check, X as XIcon, CornerLeftUp } from 'lucide-react'
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
import { PaneHead } from '../ui/Panes'
import './tasks.css'
import { t } from '~/strings'
import { MarkdownViewer } from '../MarkdownViewer'

interface Props {
  task: TaskItem | null
  allTasks: TaskItem[]
  /** Closes the task (back below 1024 px, the X when the board or timeline is behind it). */
  onClose: () => void
  /** Opens the details pane as a side sheet; shown only below 1400 px. */
  onDetails?: () => void
  onUpdate: (id: string, patch: Record<string, unknown>) => Promise<void>
  /** Brings an archived task back. */
  onRestore?: (id: string) => Promise<void>
  /** Archives the task; `purge` deletes an archived one for good. */
  onDelete: (id: string, cascade: boolean, purge?: boolean) => Promise<void>
  onSelectTask: (id: string) => void
  onCreateSubtask: (parent: TaskItem, title: string) => Promise<void>
  busy: boolean
}

const DATE_INPUT_CLASS = 'a-input'

export function TaskDetailMain({
  task,
  allTasks,
  onClose,
  onDetails,
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
  const [menuOpen, setMenuOpen] = useState(false)

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
    setMenuOpen(false)
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

  const label = (text: string) => <SectionLabel>{text}</SectionLabel>

  return (
    <>
      <PaneHead
        title={task.title}
        back={
          <button type="button" className="a-ib a-back" onClick={onClose} aria-label={t('tasks_back')}>
            <ArrowLeft className="a-ic" aria-hidden="true" />
          </button>
        }
      >
        {editing ? (
          <>
            <button type="button" onClick={handleSaveEdit} disabled={busy || isInvalidEstimate(draftEstimate)} className="a-ib" aria-label={t('quick_note_save')}>
              <Check className="a-ic" aria-hidden="true" />
            </button>
            <button type="button" onClick={() => setEditing(false)} className="a-ib" aria-label={t('dl_btn_cancel')}>
              <XIcon className="a-ic" aria-hidden="true" />
            </button>
          </>
        ) : (
          !archived && (
            <button type="button" onClick={() => setEditing(true)} className="a-ib" aria-label={t('note_editor_edit')}>
              <Pencil className="a-ic" aria-hidden="true" />
            </button>
          )
        )}
        <div className="tk-menu-at">
          <button type="button" className="a-ib" aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t('tasks_more_for_task')} onClick={() => setMenuOpen((v) => !v)}>
            <MoreVertical className="a-ic" aria-hidden="true" />
          </button>
          {menuOpen && (
            <div role="menu" className="a-menu tk-pop">
              {!archived && (
                <button type="button" role="menuitem" className="a-menu-item" disabled={busy} onClick={handleDeleteClick}>
                  {t('chat_archive')}
                </button>
              )}
              {archived && (
                <button type="button" role="menuitem" className="a-menu-item danger" disabled={busy} onClick={() => { setMenuOpen(false); setConfirmingDelete(true) }}>
                  {t('delete_task_act')}
                </button>
              )}
              <button type="button" role="menuitem" className="a-menu-item" onClick={() => { setMenuOpen(false); onClose() }}>
                {t('close')}
              </button>
            </div>
          )}
        </div>
        {onDetails && !editing && (
          <button type="button" className="a-ib tk-details-btn" onClick={onDetails} aria-label={t('inbox_details')}>
            <Info className="a-ic" aria-hidden="true" />
          </button>
        )}
      </PaneHead>

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

      <div className="tk-body">
        <div className="tk-stack a-pad">
          {archived && (
            <div className="a-stale" role="status">
              <span>{t('tasks_archived_notice', { when: formatLocalTime(task.archived_at ?? '') })}</span>
              {onRestore && (
                <button type="button" onClick={() => onRestore(task.id)} disabled={busy} className="a-btn compact secondary">
                  {t('tasks_restore')}
                </button>
              )}
            </div>
          )}

          <TaskVerification key={`verification-${task.id}`} task={task} onUpdate={onUpdate} />

          <div className="tk-meta">
            <span className="a-kind sm">{task.display_id}</span>
            {task.priority && <span className="a-kind sm warn">{task.priority}</span>}
            {task.long_horizon && <span className="a-kind sm">{t('task_long_running')}</span>}
            {stale && (
              <span className="a-kind sm" title={t('task_stale_hint')}>
                {t('task_stale_label')}
              </span>
            )}
          </div>

          {editing && (
            <input value={draftTitle} onChange={(e) => setDraftTitle(e.target.value)} className="a-input" aria-label={t('task_title_label')} />
          )}

          {parent && (
            <button type="button" onClick={() => onSelectTask(parent.id)} className="a-btn compact quiet">
              <CornerLeftUp className="a-ic xs" aria-hidden="true" />
              {t('task_drawer_sub_of')} {parent.display_id} {parent.title}
            </button>
          )}

          <div className="tk-pri" role="group" aria-label={t('tasks_status_label')}>
            {STATUS_ORDER.map((status: TaskStatus) => (
              <button
                key={status}
                type="button"
                aria-pressed={status === task.status}
                disabled={busy || archived || status === task.status}
                onClick={() => onUpdate(task.id, { status, expected_status: task.status })}
                className="a-chip"
              >
                {STATUS_LABELS[status]}
              </button>
            ))}
          </div>

          <div>
            <SectionLabel>{t('task_drawer_description')}</SectionLabel>
            {editing ? (
              <textarea value={draftDescription} onChange={(e) => setDraftDescription(e.target.value)} rows={5} className="a-input" />
            ) : task.description ? (
              <div className="a-body">
                <MarkdownViewer content={task.description} bare />
              </div>
            ) : (
              <p className="a-body-sm a-muted">{t('task_drawer_no_description')}</p>
            )}
          </div>

          <div>
            <SectionLabel className="mb-2">{t('tasks_priority')}</SectionLabel>
            <div className="tk-pri">
              {PRIORITIES.map((p) => (
                <button key={p} type="button" aria-pressed={p === task.priority} disabled={busy} onClick={() => onUpdate(task.id, { priority: p === task.priority ? null : p })} className="a-chip">
                  {p}
                </button>
              ))}
            </div>
          </div>

          {editing && (
            <>
              <div className="tk-two">
                <DateRangeInputs start={draftStartDate} due={draftDueDate} onStart={setDraftStartDate} onDue={setDraftDueDate} inputClass={DATE_INPUT_CLASS} label={label} />
                <EstimateInput value={draftEstimate} onChange={setDraftEstimate} inputClass={DATE_INPUT_CLASS} label={label} />
              </div>
              {task.status === 'blocked' && (
                <div className="tk-two">
                  <div>
                    <SectionLabel>{t('task_drawer_why_blocked')}</SectionLabel>
                    <input value={draftReason} onChange={(e) => setDraftReason(e.target.value)} maxLength={500} placeholder={t('task_drawer_in_the_way')} className="a-input" />
                  </div>
                  <div>
                    <SectionLabel>{t('task_drawer_waiting_on')}</SectionLabel>
                    <input value={draftWaiting} onChange={(e) => setDraftWaiting(e.target.value)} maxLength={500} placeholder={t('task_drawer_waiting_hint')} className="a-input" />
                  </div>
                </div>
              )}
              <label className="l-choice">
                <input type="checkbox" checked={draftLong} onChange={(e) => setDraftLong(e.target.checked)} />
                {t('task_drawer_long_running')}
              </label>
              <div>
                <SectionLabel>{t('task_drawer_assigned_label')}</SectionLabel>
                <input value={draftAssignees} onChange={(e) => setDraftAssignees(e.target.value)} placeholder={t('task_drawer_assigned_hint')} className="a-input" />
              </div>
              <div>
                <SectionLabel>{t('task_drawer_tags_label')}</SectionLabel>
                <input value={draftTags} onChange={(e) => setDraftTags(e.target.value)} placeholder={t('task_drawer_tags_hint')} className="a-input" />
              </div>
            </>
          )}

          {!editing && task.status === 'blocked' && (task.blocked_reason || task.waiting_on) && (
            <div className="a-card">
              {task.blocked_reason && <p className="a-body-sm-b">{t('task_blocked_line', { reason: task.blocked_reason })}</p>}
              {task.waiting_on && <p className="a-body-sm a-muted">{t('task_waiting_on', { ids: task.waiting_on })}</p>}
              {task.blocked_since && <p className="a-body-sm a-muted">{t('task_blocked_since', { when: formatLocalTime(task.blocked_since) })}</p>}
            </div>
          )}

          {!editing && (task.tags.length > 0 || task.assignees.length > 0) && (
            <div className="tk-meta">
              {task.assignees.length > 0 && (
                <span className="a-kind" title={t('task_assigned_to')}>
                  {t('task_for', { who: task.assignees.join(', ') })}
                </span>
              )}
              {task.tags.map((tag) => (
                <span key={tag} className="a-kind">
                  {tag}
                </span>
              ))}
            </div>
          )}

          <TaskRelations task={task} allTasks={allTasks} busy={busy} onUpdate={onUpdate} onSelectTask={onSelectTask} onCreateSubtask={onCreateSubtask} />
          <TaskDecisions key={`decisions-${task.id}`} taskId={task.id} />
          <TaskCheckpoint key={`checkpoint-${task.id}`} task={task} />
          <TaskTime key={`time-${task.id}`} task={task} />
          <TaskSessions key={`sessions-${task.id}`} taskId={task.id} taskTitle={task.title} done={archived || task.status === 'complete'} />
          <TaskComments key={`comments-${task.id}`} taskId={task.id} />
        </div>
      </div>
    </>
  )
}

/** The details beside the open task: its dates, links and history. Read only; editing is in the task itself. */
export function TaskDetailSide({ task, onClose }: { task: TaskItem; onClose?: () => void }) {
  const row = (k: string, v: string | null) =>
    v ? (
      <div className="a-drow" key={k}>
        <span>{k}</span>
        <span>{v}</span>
      </div>
    ) : null
  return (
    <>
      <PaneHead title={t('inbox_details')}>
        {onClose && (
          <button type="button" className="a-ib" onClick={onClose} aria-label={t('close')}>
            <XIcon className="a-ic" aria-hidden="true" />
          </button>
        )}
      </PaneHead>
      <div className="tk-body">
        <div className="tk-stack a-pad">
          <div className="a-card l-lines">
            {row(t('task_start'), task.start_date)}
            {row(t('task_due'), task.due_date)}
            {row(t('task_work_started'), task.work_started_at ? formatLocalTime(task.work_started_at) : null)}
            {row(t('task_first_ready'), task.first_ready_for_review_at ? formatLocalTime(task.first_ready_for_review_at) : null)}
            {row(t('task_completed'), task.completed_at ? formatLocalTime(task.completed_at) : null)}
          </div>
          <TaskLinks key={`links-${task.id}`} task={task} />
          <TaskActivity key={`activity-${task.id}`} task={task} />
        </div>
      </div>
    </>
  )
}
