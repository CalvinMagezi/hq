import { AlertTriangle, Check, Circle } from 'lucide-react'
import type { Rollout } from '~/lib/verificationApi'
import { rolloutView } from './verification'
import { t } from '~/strings'
import '../leaf.css'

/** Where verification is in being turned on, and what the next step waits for. Read only: the owner changes the setting. */
export function RolloutPanel({ rollout }: { rollout: Rollout }) {
  const v = rolloutView(rollout)
  return (
    <div className="l-stack">
      <p className="a-label">{t('verify_rollout_title')}</p>
      <p className="a-body-sm">{v.stage}</p>
      {v.warnings.map((w) => (
        <p key={w} className="a-why">
          <AlertTriangle className="a-ic xs" aria-hidden="true" />
          <span>{w}</span>
        </p>
      ))}
      {v.next && (
        <div className="l-stack">
          <p className="a-body-sm">
            Next: {v.next.label}. <span className="a-muted">{v.next.verdict}</span>
          </p>
          {v.gates.length > 0 && (
            <ul className="l-stack">
              {v.gates.map((g) => (
                <li key={g.label} className="l-row a-body-sm">
                  {g.met ? (
                    <Check className="a-ic xs" aria-hidden="true" />
                  ) : (
                    <Circle className="a-ic xs a-warn-t" aria-hidden="true" />
                  )}
                  <span className={g.met ? 'a-muted' : ''}>
                    {g.label}. <span className="a-muted">{g.detail}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
          <p className="a-cap a-muted">
            {t('verify_rollout_setting')} <code className="a-mono">{v.next.change}</code>
          </p>
        </div>
      )}
    </div>
  )
}
