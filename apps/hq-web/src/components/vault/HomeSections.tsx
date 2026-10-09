import { Link } from '@tanstack/react-router'
import type { PinnedNote, RecentFile } from '~/lib/vaultApi'
import { STATUS_LABELS, type TaskItem } from '~/lib/tasksApi'
import type { SectionState } from '~/lib/homeSections'
import { relTime } from '~/lib/time'
import { SignalCard } from '~/components/SignalCard'
import { PinnedCard } from './PinnedCard'
import { HomeSection } from './HomeSection'

interface HomeSectionsProps {
  pinned: SectionState<PinnedNote>
  recent: SectionState<RecentFile>
  tasks: SectionState<TaskItem>
  onRetryPinned: () => void
  onRetryRecent: () => void
  onRetryTasks: () => void
}

function TaskRow({ task }: { task: TaskItem }) {
  return (
    <Link
      to="/tasks"
      search={{ task: task.id }}
      className="glass-card rounded-xl px-3.5 py-3 flex items-center gap-3 min-w-0"
    >
      <span className="text-[11px] font-bold shrink-0" style={{ color: 'var(--accent-green)' }}>
        {task.display_id}
      </span>
      <span className="text-[12px] truncate flex-1 min-w-0" style={{ color: 'var(--text-primary)' }}>
        {task.title}
      </span>
      <span className="text-[9px] shrink-0" style={{ color: 'var(--text-dim)' }}>
        {STATUS_LABELS[task.status]} · {relTime(task.updated_at)}
      </span>
    </Link>
  )
}

/** Pinned notes, then recent project notes, then in-progress tasks. */
export function HomeSections(p: HomeSectionsProps) {
  return (
    <>
      <HomeSection
        title="Pinned"
        accent="var(--accent-blue)"
        state={p.pinned}
        emptyText="No pinned notes yet. Pin a note from the vault to keep it here."
        onRetry={p.onRetryPinned}
        render={(notes) => (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {notes.map((note, i) => (
              <div key={note.path} className="stagger-item" style={{ animationDelay: `${i * 60}ms` }}>
                <PinnedCard note={note} isSelected={false} />
              </div>
            ))}
          </div>
        )}
      />
      <HomeSection
        title="Recent Project Notes"
        state={p.recent}
        emptyText="No project notes found."
        onRetry={p.onRetryRecent}
        render={(notes) => (
          <div className="grid grid-cols-1 min-[400px]:grid-cols-2 lg:grid-cols-3 gap-3">
            {notes.map((note) => <SignalCard key={note.path} note={note} lane="note" />)}
          </div>
        )}
      />
      <HomeSection
        title="In Progress Tasks"
        accent="var(--accent-green)"
        state={p.tasks}
        emptyText="No tasks in progress."
        onRetry={p.onRetryTasks}
        render={(tasks) => (
          <div className="flex flex-col gap-2">
            {tasks.map((t) => <TaskRow key={t.id} task={t} />)}
          </div>
        )}
      />
    </>
  )
}
