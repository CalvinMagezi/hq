import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { addTrustedDomain, fetchTrustedSites, forgetTrustedSite } from '~/lib/notificationsApi'
import { t } from '~/strings'

const KEY = ['fetch-sites']

/** Sites that stopped asking: ones the owner named, and ones allowed in several chats. */
export function TrustedSites() {
  const client = useQueryClient()
  const [draft, setDraft] = useState('')
  const [error, setError] = useState('')
  const data = useQuery({ queryKey: KEY, queryFn: fetchTrustedSites }).data
  const refresh = () => client.invalidateQueries({ queryKey: KEY })
  const remove = async (site: string) => {
    await forgetTrustedSite(site)
    await refresh()
  }
  const add = async () => {
    try {
      await addTrustedDomain(draft)
      setDraft('')
      setError('')
      await refresh()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  const row = (site: string) => (
    <li key={site} className="a-li">
      <div className="a-li-main">
        <b>{site}</b>
      </div>
      <button type="button" className="a-btn secondary compact" onClick={() => void remove(site)}>
        {t('sites_trusted_remove')}
      </button>
    </li>
  )
  const earned = data?.sites ?? []
  const domains = data?.domains ?? []
  return (
    <>
      <section className="a-card mx-4 mb-3">
        <b className="a-body-b">{t('sites_domains_title')}</b>
        <p className="a-body-sm">{t('sites_domains_body')}</p>
        <form
          onSubmit={(e) => {
            e.preventDefault()
            void add()
          }}
        >
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={t('sites_domains_placeholder')}
            aria-label={t('sites_domains_title')}
          />
          <button type="submit" className="a-btn compact" disabled={!draft.trim()}>
            {t('sites_domains_add')}
          </button>
        </form>
        {error && <p className="a-body-sm">{error}</p>}
        <ul>{domains.map(row)}</ul>
      </section>
      {earned.length > 0 && (
        <section className="a-card mx-4 mb-3">
          <b className="a-body-b">{t('sites_trusted_title')}</b>
          <p className="a-body-sm">{t('sites_trusted_body')}</p>
          <ul>{earned.map(row)}</ul>
        </section>
      )}
    </>
  )
}
