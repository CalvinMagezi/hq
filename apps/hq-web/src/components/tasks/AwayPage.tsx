import { Link } from '@tanstack/react-router'
import { t } from '~/strings'
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
    <section aria-label={t('tasks_away_title')} className="space-y-5 pb-8">
      <div>
        <h2 className="text-lg font-semibold text-white">{t('tasks_away_title')}</h2>
        <p className="text-xs text-neutral-500">{t('tasks_away_window', { hours: AWAY_HOURS })}</p>
      </div>
      {sections.length === 0 && <p className="text-xs text-neutral-400">{t('tasks_away_nothing')}</p>}
      {sections.map((s) => (
        <div key={s.id} data-away-section={s.id}>
          <h3 className="text-xs font-semibold text-neutral-500 uppercase tracking-wider mb-1.5">
            {s.title} {s.count}
          </h3>
          {s.id === 'spot' ? (
            <button type="button" onClick={onAudit} className="hq-btn-ghost !h-10 !px-3 !text-xs">
              {t('tasks_away_spot_ask')}
            </button>
          ) : (
            <ul className="space-y-1.5">
              {s.rows.map((r) => (
                <li key={r.key}>
                  {r.taskId ? (
                    <button
                      type="button"
                      onClick={() => onOpenTask(r.taskId!)}
                      className="w-full text-left min-h-11 px-3 py-2 rounded-xl bg-white/[0.03] border border-white/10 hover:bg-white/[0.06]"
                    >
                      <span className="block text-xs text-neutral-100 truncate">{r.title}</span>
                      {r.detail && <span className="block text-xs text-neutral-400 truncate">{r.detail}</span>}
                    </button>
                  ) : (
                    <span className="block text-xs text-neutral-300 px-1">{r.title}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      ))}
      <Link to="/tasks" className="inline-block text-xs text-emerald-400 hover:text-emerald-300">
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
    return <p className="text-xs text-neutral-400">{t('tasks_away_nothing')}</p>
  }
  return <AwayPageView briefing={b} nowMs={Date.now()} owner={owner} onOpenTask={onOpenTask} onAudit={onAudit} />
}
