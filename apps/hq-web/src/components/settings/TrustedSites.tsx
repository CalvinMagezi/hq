import { useQuery, useQueryClient } from '@tanstack/react-query'
import { fetchTrustedSites, forgetTrustedSite } from '~/lib/notificationsApi'
import { t } from '~/strings'

const KEY = ['fetch-sites']

/** Sites that stopped asking because the owner allowed them in several chats. */
export function TrustedSites() {
  const client = useQueryClient()
  const sites = useQuery({ queryKey: KEY, queryFn: fetchTrustedSites }).data ?? []
  if (sites.length === 0) return null
  const remove = async (site: string) => {
    await forgetTrustedSite(site)
    await client.invalidateQueries({ queryKey: KEY })
  }
  return (
    <section className="a-card mx-4 mb-3">
      <b className="a-body-b">{t('sites_trusted_title')}</b>
      <p className="a-body-sm">{t('sites_trusted_body')}</p>
      <ul>
        {sites.map((site) => (
          <li key={site} className="a-li">
            <div className="a-li-main">
              <b>{site}</b>
            </div>
            <button type="button" className="a-btn secondary compact" onClick={() => void remove(site)}>
              {t('sites_trusted_remove')}
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}
