import { capabilityModeKey, type ParsedSandboxGrant } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const REASON_LIMIT = 300

export interface SandboxGrantCardProps {
  grant: ParsedSandboxGrant
  busy?: boolean
  error?: string | null
  onAllow: () => void
  onDecline: () => void
  /** False when the card sits inside an article that already renders the title header. */
  showTitle?: boolean
}

/** Owner decision card for a sandbox-grants request: exact resource and mode, task, expiry, and helper's words clearly separated from HQ's. */
export function SandboxGrantCard({ grant, busy, error, onAllow, onDecline, showTitle = true }: SandboxGrantCardProps) {
  const mode = t(capabilityModeKey(grant.capability))
  const reason = grant.helperReason.trim()
  const clippedReason = reason.length > REASON_LIMIT ? `${reason.slice(0, REASON_LIMIT)}...` : reason

  return (
    <section className="a-card" aria-label={grant.title || t('grants_title')}>
      {showTitle && <p className="text-xs font-semibold">{grant.title || t('grants_title')}</p>}
      <blockquote className="a-quote" data-testid="grant-resource">
        <span className="a-cap a-muted">{mode}</span>{' '}
        <code className="a-mono">{grant.resource}</code>
      </blockquote>
      <div className="flex flex-col gap-1">
        {grant.taskId && (
          <div className="a-drow">
            <span>{t('grants_task_label')}</span>
            <span className="a-mono">{grant.taskId}</span>
          </div>
        )}
        <p className="a-cap a-muted" data-testid="grant-expiry">
          {t('grants_expires_day')}
        </p>
      </div>
      {clippedReason && (
        <div className="flex flex-col gap-1" data-testid="grant-helper-reason">
          <p className="a-cap a-muted">{t('grants_helper_words')}</p>
          <blockquote className="a-quote a-muted">{clippedReason}</blockquote>
        </div>
      )}
      <div className="a-dlg-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <button type="button" className="a-btn secondary" disabled={busy} onClick={onDecline}>
          {t('grants_decline')}
        </button>
        <button
          type="button"
          className="a-btn secondary"
          disabled={busy}
          aria-busy={busy || undefined}
          onClick={onAllow}
        >
          {busy ? t('approval_answering') : t('grants_allow')}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  )
}
