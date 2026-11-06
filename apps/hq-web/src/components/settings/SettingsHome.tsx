import { Link } from '@tanstack/react-router'
import { ChevronRight } from 'lucide-react'
import { settingsGroups, type WhereHqRuns } from '~/lib/settingsGroups'
import { t } from '~/strings'

interface Props extends WhereHqRuns {
  /** The words at the right of a row, by row id. */
  trailing: Partial<Record<string, string>>
  /** Rows shown amber, by row id (H4). */
  needsLook?: string[]
  onSignOut: () => void
  /** The window shows HQ on a paired phone: these are the phone's settings. */
  phone?: boolean
}

/** The four groups as a list; at every width it is the index of Settings. */
export function SettingsHome({ lite, hub, phone, trailing, needsLook = [], onSignOut }: Props) {
  return (
    <div className="flex flex-col gap-5 pb-6">
      {phone && (
        <div className="flex flex-col gap-1 px-4">
          <p className="a-body">{t('hub_note_phone')}</p>
          <p className="a-muted a-body-sm">{t('hub_note_phone_own')}</p>
        </div>
      )}
      {settingsGroups({ lite, hub }).map((group) => (
        <section key={group.id} aria-labelledby={`group-${group.id}`}>
          <h2 id={`group-${group.id}`} className="a-label px-4">
            {t(group.label)}
          </h2>
          {group.id === 'browser' && hub && <p className="a-muted a-body-sm px-4">{t('hub_shown_by')}</p>}
          <div className="a-set">
            {group.rows.map((row) => {
              const inner = (
                <>
                  <div className="a-li-main">
                    <b>{t(row.label)}</b>
                    {trailing[row.id] && <span className={needsLook.includes(row.id) ? 'a-warn-t' : undefined}>{trailing[row.id]}</span>}
                  </div>
                  <ChevronRight className="a-ic a-muted" aria-hidden="true" />
                </>
              )
              return row.path ? (
                <Link key={row.id} to={row.path} className="a-li" activeProps={{ 'aria-current': 'true' }}>
                  {inner}
                </Link>
              ) : (
                <button key={row.id} type="button" className="a-li" onClick={onSignOut}>
                  {inner}
                </button>
              )
            })}
          </div>
        </section>
      ))}
    </div>
  )
}
