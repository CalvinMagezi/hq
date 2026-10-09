import { Link } from '@tanstack/react-router'
import { Loader2, Terminal } from 'lucide-react'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { sessionTitle } from '~/lib/workbench'
import { SessionBadges } from '../sessions/SessionRow'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'

const TASK_SESSIONS_POLL_MS = 15_000

/** The coding-agent sessions launched for or linked to this task, each linking to the Workbench. */
export function TaskSessions({ taskId }: { taskId: string }) {
  const sessions = usePolled(taskId, () => globalSessionsApi.list({ task_id: taskId }), TASK_SESSIONS_POLL_MS)
  return (
    <section aria-label="Linked agents">
      <SectionLabel>Agents</SectionLabel>
      {sessions.loading ? (
        <div role="status" className="flex items-center gap-2 text-xs font-mono text-neutral-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading agents
        </div>
      ) : !sessions.data ? (
        <p role="alert" className="text-xs font-mono text-rose-400">
          {sessions.error ?? 'Could not load agents.'}
        </p>
      ) : sessions.data.length === 0 ? (
        <p className="text-xs font-mono text-neutral-500">No agents on this task yet.</p>
      ) : (
        <ul className="space-y-1.5">
          {sessions.data.map((s) => (
            <li key={s.id}>
              <Link
                to="/sessions"
                search={{ id: s.id }}
                className="block px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5 hover:bg-white/5 space-y-0.5"
              >
                <span className="flex items-center gap-2 min-w-0 text-xs font-mono text-neutral-200">
                  <Terminal className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
                  <span className="truncate">
                    {sessionTitle(s)}
                  </span>
                </span>
                <SessionBadges session={s} />
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
