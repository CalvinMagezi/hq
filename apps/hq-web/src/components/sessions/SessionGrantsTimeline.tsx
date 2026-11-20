import { grantStateKey, isResourceGrant, type SandboxGrant } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

export interface SessionGrantsTimelineProps {
  grants: SandboxGrant[]
  revokingId: string | null
  error?: string | null
  onRevoke: (id: string) => void
}

/** Quiet timeline rows for a session's access grants: policy allows with Revoke, and muted history rows with decision notes. */
export function SessionGrantsTimeline({ grants, revokingId, error, onRevoke }: SessionGrantsTimelineProps) {
  const rows = grants.filter((g) => isResourceGrant(g) && g.state !== 'pending')
  if (rows.length === 0) return null

  return (
    <section aria-label={t('grants_timeline_title')} className="a-body-sm a-muted flex flex-col gap-2">
      <b>{t('grants_timeline_title')}</b>
      {error && (
        <p role="alert" className="a-cap a-warn-t">
          {error}
        </p>
      )}
      <ul className="flex flex-col gap-1">
        {rows.map((g) => {
          const active = g.state === 'active'
          if (active) {
            const label =
              g.decided_by === 'policy'
                ? t('grants_hq_allowed', { resource: g.resource })
                : t('grants_owner_allowed', { resource: g.resource })
            const busy = revokingId === g.id
            return (
              <li key={g.id} className="a-drow" data-testid="grant-active-row">
                <span className="a-mono">{label}</span>
                <button
                  type="button"
                  className="a-btn quiet danger compact"
                  disabled={revokingId !== null}
                  aria-busy={busy || undefined}
                  onClick={() => onRevoke(g.id)}
                >
                  {busy ? t('grants_revoking') : t('grants_revoke')}
                </button>
              </li>
            )
          }
          const stateLabel = t(grantStateKey(g.state))
          return (
            <li key={g.id} className="a-drow a-muted" data-testid="grant-muted-row">
              <span>
                {stateLabel}: <span className="a-mono">{g.resource}</span>
                {g.decision_note ? ` (${g.decision_note})` : ''}
              </span>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
