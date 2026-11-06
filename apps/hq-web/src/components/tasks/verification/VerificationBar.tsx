import { useState } from 'react'
import { ChevronDown, ClipboardCheck, Gauge, ShieldCheck } from 'lucide-react'
import { t } from '~/strings'
import '../leaf.css'
import { fetchVerificationStatus, type VerificationStatus } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../../sessions/usePolled'
import { QueueHealthPanel } from './QueueHealthPanel'
import { verificationActive, waitingChips } from './verification'

const STATUS_POLL_MS = 30_000

interface ViewProps {
  status: VerificationStatus
  healthOpen: boolean
  onToggleHealth: () => void
  onVerifyNow: () => void
  onAudit: () => void
}

/** Counts by who tasks wait on, and the three things the owner does here: verify now, audit, read the queue. */
export function VerificationBarView({ status, healthOpen, onToggleHealth, onVerifyNow, onAudit }: ViewProps) {
  const chips = waitingChips(status.queue.backlog)
  const watching = status.mode === 'shadow'
  const audits = status.calibration.open_audits
  return (
    <section aria-label={t('check_ready_to_check')} className="a-card mb-4">
      <div className="l-bar">
        <div className="l-row l-grow">
          <ShieldCheck className={`a-ic sm ${watching ? 'a-warn-t' : ''}`} aria-hidden="true" />
          <span className="a-body-sm-b">{t('check_ready_to_check')}</span>
          {watching && (
            <span className="a-state needs" title={t('check_watching_title')}>
              {t('check_watching_only')}
            </span>
          )}
        </div>
        <ul className="l-bar">
          {chips.length === 0 && <li className="a-cap a-muted">{t('check_nothing_waiting')}</li>}
          {chips.map((c) => (
            <li
              key={c.who}
              className={c.yours ? 'a-state needs' : 'a-kind sm'}
            >
              {c.count} {c.label.toLowerCase()}
            </li>
          ))}
        </ul>
        <div className="l-bar">
          <button type="button" onClick={onVerifyNow} className="a-btn compact">
            <ClipboardCheck className="a-ic xs" aria-hidden="true" />
            {t('check_check_them_now')}
          </button>
          <button type="button" onClick={onAudit} className="a-btn secondary compact">
            {audits > 0 ? t('check_audit_count', { n: audits }) : t('check_audit')}
          </button>
          <button
            type="button"
            onClick={onToggleHealth}
            aria-expanded={healthOpen}
            aria-label="Queue health"
            className="a-btn secondary compact"
          >
            <Gauge className="a-ic xs" aria-hidden="true" />
            <ChevronDown className={`a-ic xs ${healthOpen ? 'rotate-180' : ''}`} aria-hidden="true" />
          </button>
        </div>
      </div>
      {healthOpen && <QueueHealthPanel health={status.queue} rollout={status.rollout} />}
    </section>
  )
}

interface Props {
  onVerifyNow: () => void
  onAudit: () => void
}

/** Shown on the tasks page once verification is on; nothing at all while it is off. */
export function VerificationBar({ onVerifyNow, onAudit }: Props) {
  const status = usePolled('verification-status', fetchVerificationStatus, STATUS_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], status.refresh)
  const [healthOpen, setHealthOpen] = useState(false)
  if (!status.data || !verificationActive(status.data.mode)) return null
  return (
    <VerificationBarView
      status={status.data}
      healthOpen={healthOpen}
      onToggleHealth={() => setHealthOpen((v) => !v)}
      onVerifyNow={onVerifyNow}
      onAudit={onAudit}
    />
  )
}
