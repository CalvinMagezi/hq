import { Link } from '@tanstack/react-router'
import { fetchTasksLinkedToClient, STATUS_LABELS } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { useRefreshOn } from '~/lib/useRefreshOn'

const NOTE_TASKS_POLL_MS = 300_000

/** The tasks that started from this note. Renders nothing for a note with none. */
export function NoteTasks({ path }: { path: string }) {
  const found = usePolled(path, async () => (await fetchTasksLinkedToClient('vault_note', path)).tasks, NOTE_TASKS_POLL_MS)
  useRefreshOn(['task:sync'], found.refresh)
  if (!found.data || found.data.length === 0) return null
  return (
    <section aria-label="Tasks from this note" className="mt-8 pt-4 border-t border-white/10">
      <h3 className="text-[11px] font-mono font-semibold uppercase tracking-wider mb-2" style={{ color: 'var(--text-dim)' }}>
        Tasks from this note
      </h3>
      <ul className="space-y-1.5">
        {found.data.map((task) => (
          <li key={task.id}>
            <Link
              to="/tasks"
              search={{ task: task.id }}
              className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5 hover:bg-white/5 text-xs font-mono min-w-0"
            >
              <span className="text-[10px] text-neutral-500 shrink-0">{task.display_id}</span>
              <span className="truncate flex-1 text-neutral-200">{task.title}</span>
              <span className="text-[10px] text-neutral-500 shrink-0">{STATUS_LABELS[task.status]}</span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
