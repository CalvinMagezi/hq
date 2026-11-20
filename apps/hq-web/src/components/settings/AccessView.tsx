import {
  activeResourceGrants,
  capabilityModeKey,
  grantStateKey,
  historyResourceGrants,
  type SandboxGrant,
  type SandboxGrantProfile,
} from '~/lib/sandboxGrantsApi'
import { unixToMs } from '~/lib/connectorApi'
import { dateByAge } from '~/lib/time'
import { t } from '~/strings'

export interface AccessViewProps {
  grants: SandboxGrant[]
  profiles: SandboxGrantProfile[]
  canChange: boolean
  revokingId: string | null
  error?: string | null
  onRevoke: (id: string) => void
  nowMs?: number
}

export function profileAllowLines(profile: SandboxGrantProfile): string[] {
  const lines: string[] = []
  if (profile.writable.length > 0) lines.push(t('grants_profile_writable', { items: profile.writable.join(', ') }))
  if (profile.readable.length > 0) lines.push(t('grants_profile_readable', { items: profile.readable.join(', ') }))
  if (profile.loopback_ports.length > 0) lines.push(t('grants_profile_ports', { items: profile.loopback_ports.join(', ') }))
  if (profile.egress.length > 0) lines.push(t('grants_profile_egress', { items: profile.egress.join(', ') }))
  return lines
}

/** Settings > Access: active grants with Revoke, read-only configured profiles, and grant history. */
export function AccessView({ grants, profiles, canChange, revokingId, error, onRevoke, nowMs = Date.now() }: AccessViewProps) {
  const active = activeResourceGrants(grants)
  const history = historyResourceGrants(grants)

  return (
    <div className="flex flex-col gap-4">
      {error && (
        <p role="alert" className="a-body-sm a-warn-t">
          {error}
        </p>
      )}

      <section aria-labelledby="access-active-head" className="flex flex-col gap-2">
        <h3 id="access-active-head" className="a-grp">
          {t('grants_settings_active')}
        </h3>
        {active.length === 0 ? (
          <p className="a-body-sm a-muted">{t('grants_settings_active_empty')}</p>
        ) : (
          <div className="a-set">
            {active.map((g) => {
              const busy = revokingId === g.id
              const decided = g.decided_by === 'policy' ? t('grants_by_policy') : t('grants_by_owner')
              const meta = [t(capabilityModeKey(g.capability)), decided, g.host, dateByAge(unixToMs(g.requested_at), nowMs)]
                .filter(Boolean)
                .join(' · ')
              return (
                <div key={g.id} className="a-li" data-testid="access-active-grant">
                  <div className="a-li-main">
                    <b className="a-mono">{g.resource}</b>
                    <span>{meta}</span>
                    {!canChange && <span>{t('settings_folders_revoke_needs')}</span>}
                  </div>
                  <button
                    type="button"
                    className="a-btn quiet danger"
                    disabled={revokingId !== null || !canChange}
                    aria-busy={busy || undefined}
                    onClick={() => onRevoke(g.id)}
                  >
                    {busy ? t('grants_revoking') : t('grants_revoke')}
                  </button>
                </div>
              )
            })}
          </div>
        )}
      </section>

      <section aria-labelledby="access-profiles-head" className="flex flex-col gap-2">
        <h3 id="access-profiles-head" className="a-grp">
          {t('grants_settings_profiles')}
        </h3>
        <p className="a-body-sm a-muted">{t('grants_settings_profiles_note')}</p>
        {profiles.length === 0 ? (
          <p className="a-body-sm a-muted">{t('grants_settings_profiles_empty')}</p>
        ) : (
          <div className="a-set">
            {profiles.map((p) => {
              const lines = profileAllowLines(p)
              return (
                <div key={p.name} className="a-li" data-testid="access-profile-row">
                  <div className="a-li-main">
                    <b>{p.name}</b>
                    {lines.length === 0 ? (
                      <span>{t('grants_profile_empty')}</span>
                    ) : (
                      lines.map((line) => (
                        <span key={line} className="a-mono">
                          {line}
                        </span>
                      ))
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </section>

      <section aria-labelledby="access-history-head" className="flex flex-col gap-2">
        <h3 id="access-history-head" className="a-grp">
          {t('grants_settings_history')}
        </h3>
        {history.length === 0 ? (
          <p className="a-body-sm a-muted">{t('grants_settings_history_empty')}</p>
        ) : (
          <div className="a-set">
            {history.map((g) => {
              const stateWord = t(grantStateKey(g.state))
              const detail = [stateWord, t(capabilityModeKey(g.capability)), dateByAge(unixToMs(g.requested_at), nowMs)]
                .filter(Boolean)
                .join(' · ')
              return (
                <div key={g.id} className="a-li a-muted" data-testid="access-history-row">
                  <div className="a-li-main">
                    <b className="a-mono">{g.resource}</b>
                    <span>{detail}</span>
                    {g.decision_note && <span>{g.decision_note}</span>}
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </section>
    </div>
  )
}
