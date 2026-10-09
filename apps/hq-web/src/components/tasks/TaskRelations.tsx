import { useMemo, useState } from 'react'
import { Check, Plus, X } from 'lucide-react'
import { STATUS_LABELS, type TaskItem } from '~/lib/tasksApi'
import { STATUS_BADGE_CLASS } from './TaskCard'

interface Props {
  task: TaskItem
  allTasks: TaskItem[]
  busy: boolean
  onUpdate: (id: string, patch: Record<string, unknown>) => Promise<void>
  onSelectTask: (id: string) => void
  onCreateSubtask: (parent: TaskItem, title: string) => Promise<void>
}

const SECTION_TITLE_CLASS = 'text-xs font-semibold text-neutral-400 uppercase tracking-wider mb-2'
const INPUT_CLASS =
  'flex-1 px-3 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400'

function TaskLine({ task, onOpen, children }: { task: TaskItem; onOpen: () => void; children?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5">
      {children}
      <button type="button" onClick={onOpen} className="flex items-center gap-2 min-w-0 flex-1 text-left">
        <span className="text-[11px] text-neutral-500 shrink-0">{task.display_id}</span>
        <span className={`text-xs truncate ${task.status === 'complete' ? 'text-neutral-500 line-through' : 'text-neutral-200'}`}>
          {task.title}
        </span>
      </button>
      <span className={`text-[9px] uppercase px-1.5 py-0.5 rounded-full border shrink-0 ${STATUS_BADGE_CLASS[task.status]}`}>
        {STATUS_LABELS[task.status]}
      </span>
    </div>
  )
}

export function TaskRelations({ task, allTasks, busy, onUpdate, onSelectTask, onCreateSubtask }: Props) {
  const [subtaskTitle, setSubtaskTitle] = useState('')
  const [dependencyQuery, setDependencyQuery] = useState('')

  const byId = useMemo(() => new Map(allTasks.map((t) => [t.id, t])), [allTasks])
  const subtasks = useMemo(
    () => allTasks.filter((t) => t.parent_task_id === task.id).sort((a, b) => a.display_id.localeCompare(b.display_id)),
    [allTasks, task.id]
  )
  const waitingOn = task.depends_on.map((id) => byId.get(id)).filter((t): t is TaskItem => Boolean(t))
  const blocking = useMemo(() => allTasks.filter((t) => t.depends_on.includes(task.id)), [allTasks, task.id])
  const candidates = allTasks.filter((t) => t.id !== task.id && !task.depends_on.includes(t.id))

  const addSubtask = async () => {
    const title = subtaskTitle.trim()
    if (!title) return
    await onCreateSubtask(task, title)
    setSubtaskTitle('')
  }

  const addDependency = async () => {
    const displayId = dependencyQuery.trim().split(/\s+/)[0]?.toUpperCase()
    const match = candidates.find((t) => t.display_id.toUpperCase() === displayId)
    if (!match) return
    await onUpdate(task.id, { add_depends_on: [match.id] })
    setDependencyQuery('')
  }

  return (
    <>
      {!task.parent_task_id && (
        <div>
          <h3 className={SECTION_TITLE_CLASS}>
            Sub-tasks ({task.subtask_done}/{task.subtask_count})
          </h3>
          <div className="space-y-1.5 mb-2">
            {subtasks.map((sub) => {
              const done = sub.status === 'complete'
              return (
                <TaskLine key={sub.id} task={sub} onOpen={() => onSelectTask(sub.id)}>
                  <button
                    type="button"
                    disabled={busy}
                    title={done ? 'Reopen' : 'Mark complete'}
                    onClick={() => onUpdate(sub.id, { status: done ? 'to_do' : 'complete', expected_status: sub.status })}
                    className={`w-4 h-4 rounded border flex items-center justify-center shrink-0 ${
                      done ? 'bg-emerald-500/20 border-emerald-500/40 text-emerald-400' : 'border-white/20 text-transparent hover:border-white/40'
                    }`}
                  >
                    <Check className="w-3 h-3" />
                  </button>
                </TaskLine>
              )
            })}
          </div>
          <div className="flex items-center gap-2">
            <input
              value={subtaskTitle}
              onChange={(e) => setSubtaskTitle(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && addSubtask()}
              placeholder="Add a sub-task..."
              className={INPUT_CLASS}
            />
            <button type="button" onClick={addSubtask} disabled={busy || !subtaskTitle.trim()} className="p-1.5 rounded-lg text-emerald-400 hover:bg-white/10 disabled:opacity-40">
              <Plus className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      <div>
        <h3 className={SECTION_TITLE_CLASS}>Waiting on ({waitingOn.length})</h3>
        <div className="space-y-1.5 mb-2">
          {waitingOn.map((dep) => (
            <TaskLine key={dep.id} task={dep} onOpen={() => onSelectTask(dep.id)}>
              <button
                type="button"
                disabled={busy}
                title="Remove dependency"
                onClick={() => onUpdate(task.id, { remove_depends_on: [dep.id] })}
                className="text-neutral-500 hover:text-rose-400 shrink-0"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </TaskLine>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <input
            list={`dependency-candidates-${task.id}`}
            value={dependencyQuery}
            onChange={(e) => setDependencyQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && addDependency()}
            placeholder="Task id, e.g. FR-012"
            className={INPUT_CLASS}
          />
          <datalist id={`dependency-candidates-${task.id}`}>
            {candidates.map((t) => (
              <option key={t.id} value={`${t.display_id} ${t.title}`} />
            ))}
          </datalist>
          <button type="button" onClick={addDependency} disabled={busy || !dependencyQuery.trim()} className="p-1.5 rounded-lg text-emerald-400 hover:bg-white/10 disabled:opacity-40">
            <Plus className="w-4 h-4" />
          </button>
        </div>
      </div>

      {blocking.length > 0 && (
        <div>
          <h3 className={SECTION_TITLE_CLASS}>Blocking ({blocking.length})</h3>
          <div className="space-y-1.5">
            {blocking.map((t) => (
              <TaskLine key={t.id} task={t} onOpen={() => onSelectTask(t.id)} />
            ))}
          </div>
        </div>
      )}
    </>
  )
}
