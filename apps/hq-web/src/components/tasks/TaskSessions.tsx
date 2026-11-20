import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { Loader2, Terminal } from 'lucide-react'
import { globalSessionsApi } from '~/lib/sessionsApi'
import {
  NO_PROFILE,
  bindSandboxProfile,
  listSandboxGrants,
  taskProfileFromGrants,
} from '~/lib/sandboxGrantsApi'
import { sessionTitle } from '~/lib/workbench'
import { SessionBadges } from '../sessions/SessionRow'
import { usePolled } from '../sessions/usePolled'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { NewAgentDialog } from '../sessions/NewAgentDialog'
import { AccessProfilePicker } from './AccessProfilePicker'
import { SectionLabel } from './taskFields'
import './leaf.css'

const TASK_SESSIONS_POLL_MS = 120_000

/** The coding-agent sessions launched for or linked to this task, each linking to the Workbench. */
export function TaskSessions({ taskId, taskTitle, done }: { taskId: string; taskTitle: string; done: boolean }) {
  const [starting, setStarting] = useState(false)
  const [profile, setProfile] = useState(NO_PROFILE)
  const [profileBusy, setProfileBusy] = useState(false)
  const [profileStatus, setProfileStatus] = useState<string | null>(null)
  const [profileError, setProfileError] = useState<string | null>(null)
  const navigate = useNavigate()
  const sessions = usePolled(taskId, () => globalSessionsApi.list({ task_id: taskId }), TASK_SESSIONS_POLL_MS, true, true, ['task:sync', 'sessions:sync'])
  useRefreshOn(['session:changed', 'task:sync'], sessions.refresh)

  useEffect(() => {
    let live = true
    setProfile(NO_PROFILE)
    setProfileStatus(null)
    setProfileError(null)
    listSandboxGrants(`task:${taskId}`)
      .then((res) => {
        if (live) setProfile(taskProfileFromGrants(res.grants, taskId))
      })
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [taskId])

  const handleProfileChange = async (name: string) => {
    const prev = profile
    setProfile(name)
    setProfileBusy(true)
    setProfileStatus(null)
    setProfileError(null)
    try {
      await bindSandboxProfile({ scope: 'task', id: taskId, name })
      setProfileStatus(t('grants_profile_saved'))
    } catch (e) {
      setProfile(prev)
      setProfileError(e instanceof Error ? e.message : t('grants_profile_failed'))
    } finally {
      setProfileBusy(false)
    }
  }
  return (
    <section aria-label="Linked agents">
      <SectionLabel>Agents</SectionLabel>
      {sessions.loading ? (
        <div role="status" className="a-loading">
          <Loader2 className="a-ic xs animate-spin" aria-hidden="true" />
          Loading agents
        </div>
      ) : !sessions.data ? (
        <p role="alert" className="a-body-sm a-danger-t">
          {sessions.error ?? 'Could not load agents.'}
        </p>
      ) : sessions.data.length === 0 ? (
        <p className="a-body-sm a-muted">No agents on this task yet.</p>
      ) : (
        <ul className="a-card l-lines">
          {sessions.data.map((s) => (
            <li key={s.id}>
              <Link
                to="/helpers/$id"
                params={{ id: s.id }}
                className="a-drow l-tall l-col l-link"
              >
                <span className="l-row a-body-sm">
                  <Terminal className="a-ic xs a-muted" aria-hidden="true" />
                  <span className="l-clip">
                    {sessionTitle(s)}
                  </span>
                </span>
                <SessionBadges session={s} />
              </Link>
            </li>
          ))}
        </ul>
      )}
      {!done && (
        <div className="flex flex-col gap-2 mt-2">
          <AccessProfilePicker
            id={`task-profile-${taskId}`}
            value={profile}
            disabled={profileBusy}
            status={profileStatus}
            error={profileError}
            onChange={(name) => void handleProfileChange(name)}
          />
          <button type="button" onClick={() => setStarting(true)} className="a-btn secondary compact self-start">
            {t('helpers_start_on_task')}
          </button>
        </div>
      )}
      {starting && (
        <NewAgentDialog
          task={{ id: taskId, title: taskTitle }}
          onClose={() => setStarting(false)}
          onStarted={(id) => {
            setStarting(false)
            void navigate({ to: '/helpers/$id', params: { id } })
          }}
        />
      )}
    </section>
  )
}
