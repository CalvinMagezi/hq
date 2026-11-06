import { AlertTriangle } from 'lucide-react'
import type { QueueHealth, Rollout } from '~/lib/verificationApi'
import { RolloutPanel } from './RolloutPanel'
import { WAITING_LABEL, queueLines, waitingChips } from './verification'
import { t } from '~/strings'
import '../leaf.css'

/** Backlog by who it waits on, the sentences that matter, and why anything is deferred. */
export function QueueHealthPanel({ health, rollout }: { health: QueueHealth; rollout?: Rollout }) {
  const chips = waitingChips(health.backlog)
  const lines = queueLines(health)
  const alerts = health.alerts.length
  return (
    <div className="l-stack a-body-sm mt-3">
      {chips.length === 0 ? (
        <p className="a-muted">{t('verify_queue_empty')}</p>
      ) : (
        <ul className="l-grid">
          {chips.map((c) => (
            <li key={c.who} className={c.yours ? 'a-card needs' : 'a-card'}>
              <span className="block a-h3">{c.count}</span>
              <span className="a-cap a-muted">{c.label}</span>
            </li>
          ))}
        </ul>
      )}
      <ul className="l-stack">
        {lines.map((line, i) => (
          <li key={line} className={i < alerts ? 'a-why' : 'a-muted'}>
            {i < alerts && <AlertTriangle className="a-ic xs" aria-hidden="true" />}
            <span>{line}</span>
          </li>
        ))}
      </ul>
      {health.deferrals.length > 0 && (
        <div>
          <p className="a-label mb-1">{t('verify_queue_waiting')}</p>
          <ul className="l-stack">
            {health.deferrals.map((d) => (
              <li key={`${d.waiting_for}-${d.reason}`} className="a-muted">
                <b>{d.tasks}</b> {d.tasks === 1 ? 'task' : 'tasks'} ({WAITING_LABEL[d.waiting_for].toLowerCase()}): {d.reason}
              </li>
            ))}
          </ul>
        </div>
      )}
      {rollout && <RolloutPanel rollout={rollout} />}
    </div>
  )
}
