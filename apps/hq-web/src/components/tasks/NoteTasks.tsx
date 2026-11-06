import { Link } from '@tanstack/react-router'
import { fetchTasksLinkedToClient, STATUS_LABELS } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { useRefreshOn } from '~/lib/useRefreshOn'
import './leaf.css'

const NOTE_TASKS_POLL_MS = 300_000

/** The tasks that started from this note. Renders nothing for a note with none. */
export function NoteTasks({ path }: { path: string }) {
  const found = usePolled(path, async () => (await fetchTasksLinkedToClient('vault_note', path)).tasks, NOTE_TASKS_POLL_MS, true, true, ['task:sync', 'notes:change'])
  useRefreshOn(['task:sync'], found.refresh)
  if (!found.data || found.data.length === 0) return null
  return (
    <section aria-label="Tasks from this note" className="l-stack mt-8">
      <h3 className="a-label">
        Tasks from this note
      </h3>
      <ul className="a-card l-lines">
        {found.data.map((task) => (
          <li key={task.id}>
            <Link
              to="/tasks"
              search={{ task: task.id }}
              className="a-drow l-tall l-link a-body-sm"
            >
              <span className="a-cap a-muted shrink-0">{task.display_id}</span>
              <span className="l-clip l-grow">{task.title}</span>
              <span className="a-cap a-muted shrink-0">{STATUS_LABELS[task.status]}</span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
