import { Link } from '@tanstack/react-router'
import { t } from '~/strings'
import './leaf.css'
import { awaySections } from '~/lib/tasksAway'
import { fetchBriefing, type Briefing } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { verificationActive } from './verification/verification'

const AWAY_POLL_MS = 120_000
const AWAY_HOURS = 24

interface ViewProps {
  briefing: Briefing
  nowMs: number
  owner: boolean
  onOpenTask: (id: string) => void
  onAudit: () => void
}

export function AwayPageView({ briefing, nowMs, owner, onOpenTask, onAudit }: ViewProps) {
  const sections = awaySections(briefing, nowMs, owner)
  return (
    <section aria-label={t('tasks_away_title')} className="l-stack pb-8">
      <div>
        <h2 className="a-h3">{t('tasks_away_title')}</h2>
        <p className="a-cap a-muted">{t('tasks_away_window', { hours: AWAY_HOURS })}</p>
      </div>
      {sections.length === 0 && <p className="a-body-sm a-muted">{t('tasks_away_nothing')}</p>}
      {sections.map((s) => (
        <div key={s.id} data-away-section={s.id}>
          <h3 className="a-label mb-2">
            {s.title} {s.count}
          </h3>
          {s.id === 'spot' ? (
            <button type="button" onClick={onAudit} className="a-btn secondary compact">
              {t('tasks_away_spot_ask')}
            </button>
          ) : (
            <ul className="a-card l-lines">
              {s.rows.map((r) => (
                <li key={r.key}>
                  {r.taskId ? (
                    <button
                      type="button"
                      onClick={() => onOpenTask(r.taskId!)}
                      className="a-drow l-tall l-col l-link w-full"
                    >
                      <span className="block a-body-sm l-clip">{r.title}</span>
                      {r.detail && <span className="block a-cap a-muted l-clip">{r.detail}</span>}
                    </button>
                  ) : (
                    <span className="block a-body-sm l-pad">{r.title}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      ))}
      <Link to="/tasks" className="a-link">
        {t('tasks_away_open_tasks')}
      </Link>
    </section>
  )
}

interface Props {
  owner: boolean
  onOpenTask: (id: string) => void
  onAudit: () => void
}

/** The "While you were away" page at /tasks/away; nothing while checking is off. */
export function AwayPage({ owner, onOpenTask, onAudit }: Props) {
  const found = usePolled('verification-briefing', () => fetchBriefing(AWAY_HOURS), AWAY_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  const b = found.data
  if (!b || !verificationActive(b.mode)) {
    return <p className="a-body-sm a-muted">{t('tasks_away_nothing')}</p>
  }
  return <AwayPageView briefing={b} nowMs={Date.now()} owner={owner} onOpenTask={onOpenTask} onAudit={onAudit} />
}
