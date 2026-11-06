import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import './tasks.css'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { t, tn } from '~/strings'
import { TaskCard } from './TaskCard'
import { groupByInitiative, nestRows } from './hierarchy'

/** Which initiative sections are folded away, and how to change that. */
export interface ListSections {
  collapsed: ReadonlySet<string>
  onToggle: (name: string) => void
  /** Folds or unfolds every section named, leaving the rest alone. */
  onSetMany: (names: string[], collapsed: boolean) => void
}

/** Picking tasks for a bulk action: cards tick instead of opening, and every group gets a box. */
export interface ListSelection {
  ticked: ReadonlySet<string>
  onToggle: (task: TaskItem) => void
  onToggleMany: (tasks: TaskItem[], ticked: boolean) => void
}

interface Props {
  tasks: TaskItem[]
  allById: Map<string, TaskItem>
  initiativeById: Map<string, Initiative>
  onSelect: (task: TaskItem) => void
  sections: ListSections
  selection?: ListSelection
  /** Today groups by who it waits on instead of by list. */
  grouping?: 'list' | 'today'
  onToggleDone?: (task: TaskItem) => void
  onCheck?: (task: TaskItem) => void
  selectedId?: string | null
}

/** The Today tab's four groups, in the order the owner acts on them; empty ones are left out. */
export function todayGroups(tasks: TaskItem[]): Array<[string, TaskItem[]]> {
  const waiting = tasks.filter((x) => x.status === 'blocked' || x.status === 'ready_for_review')
  const working = tasks.filter((x) => x.status === 'in_progress')
  const due = tasks.filter((x) => x.status === 'to_do')
  const done = tasks.filter((x) => x.status === 'complete')
  return ([
    [t('tasks_group_waiting'), waiting],
    [t('tasks_status_in_progress'), working],
    [t('tasks_group_due'), due],
    [t('tasks_group_done_today'), done],
  ] as Array<[string, TaskItem[]]>).filter(([, items]) => items.length > 0)
}


export function TaskListView({ tasks, allById, initiativeById, onSelect, sections, selection, grouping = 'list', onToggleDone, onCheck, selectedId }: Props) {
  // Parents whose sub-tasks are folded away (not to be confused with the folded sections below).
  const [foldedParents, setFoldedParents] = useState<Set<string>>(new Set())
  // Done today starts folded: it is a record, not something to act on.
  const [doneOpen, setDoneOpen] = useState(false)
  const grouped = useMemo(
    () => (grouping === 'today' ? todayGroups(tasks) : groupByInitiative(tasks, initiativeById)),
    [grouping, tasks, initiativeById],
  )
  const doneName = t('tasks_group_done_today')
  const isFolded = (name: string) => (grouping === 'today' && name === doneName ? !doneOpen : sections.collapsed.has(name))
  const toggleSection = (name: string) => (grouping === 'today' && name === doneName ? setDoneOpen((v) => !v) : sections.onToggle(name))

  const toggleParent = (id: string) =>
    setFoldedParents((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const names = grouped.map(([name]) => name)
  const foldedCount = names.filter((n) => sections.collapsed.has(n)).length

  return (
    <div>
      {grouping === 'list' && names.length > 1 && (
        <div className="tk-bar">
          <button type="button" className="a-btn compact quiet" disabled={foldedCount === names.length} onClick={() => sections.onSetMany(names, true)}>
            {t('tasks_collapse_all')}
          </button>
          <button type="button" className="a-btn compact quiet" disabled={foldedCount === 0} onClick={() => sections.onSetMany(names, false)}>
            {t('tasks_expand_all')}
          </button>
        </div>
      )}
      {grouped.map(([groupName, items]) => {
        const folded = isFolded(groupName)
        const bodyId = `section-${groupName}`
        return (
          <section key={groupName}>
            <h2 className="a-grp tk-gh">
              {selection && <GroupBox name={groupName} items={items} selection={selection} disabled={folded} />}
              <button type="button" aria-expanded={!folded} aria-controls={bodyId} onClick={() => toggleSection(groupName)} className="tk-fold">
                <span>
                  {groupName} ({items.length})
                </span>
                {folded ? <ChevronRight className="a-ic sm" aria-hidden="true" /> : <ChevronDown className="a-ic sm" aria-hidden="true" />}
              </button>
            </h2>
            {!folded && (
              <div id={bodyId}>
                {/* While picking, every sub-task shows, so what is ticked is always what is on screen. */}
                {nestRows(items, allById, selection ? undefined : foldedParents).map(({ task, depth, parentLabel }) => (
                  <div key={task.id} className={depth === 1 ? 'tk-in1' : undefined}>
                    <TaskCard
                      task={task}
                      onSelect={onSelect}
                      asLink
                      parentLabel={parentLabel}
                      onToggleDone={onToggleDone}
                      onCheck={onCheck}
                      isSelected={selectedId === task.id}
                      select={selection && { ticked: selection.ticked.has(task.id), onToggle: selection.onToggle }}
                    />
                    {!selection && depth === 0 && task.subtask_count > 0 && (
                      <button type="button" onClick={() => toggleParent(task.id)} className="a-btn compact quiet tk-in1">
                        {foldedParents.has(task.id) ? <ChevronRight className="a-ic xs" aria-hidden="true" /> : <ChevronDown className="a-ic xs" aria-hidden="true" />}
                        {tn('tasks_sub_task_count', task.subtask_count)}
                      </button>
                    )}
                  </div>
                ))}
              </div>
            )}
          </section>
        )
      })}
    </div>
  )
}

interface GroupBoxProps {
  name: string
  items: TaskItem[]
  selection: ListSelection
  /** A folded section's tasks are not on screen, so they cannot be ticked from here. */
  disabled: boolean
}

/** Ticks or clears a whole initiative; half ticked shows as the dash state. */
function GroupBox({ name, items, selection, disabled }: GroupBoxProps) {
  const ref = useRef<HTMLInputElement>(null)
  const ticked = items.filter((t) => selection.ticked.has(t.id)).length
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = ticked > 0 && ticked < items.length
  }, [ticked, items.length])
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={ticked === items.length}
      onChange={(e) => selection.onToggleMany(items, e.target.checked)}
      disabled={disabled}
      aria-label={t('tasks_select_all_in', { name })}
      title={disabled ? t('tasks_expand_to_select') : undefined}
      className="tk-gbox"
    />
  )
}
