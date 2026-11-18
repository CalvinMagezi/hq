import { useState, type ReactNode } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import { t } from '~/strings'
import { searchSettings, type SettingsHit } from '~/lib/settingsSearch'
import type { WhereHqRuns } from '~/lib/settingsGroups'

interface Props extends WhereHqRuns {
  /** The groups, shown while nothing is typed. */
  children: ReactNode
  onSignOut: () => void
}

/**
 * The box over Settings. It matches the name of every row, its group and the line under it, and jumps to
 * the page. With nothing found it offers the Vault, since that is usually what the person wanted.
 */
export function SettingsFind({ lite, hub, children, onSignOut }: Props) {
  const [query, setQuery] = useState('')
  const [at, setAt] = useState(0)
  const navigate = useNavigate()
  const typed = query.trim() !== ''
  const hits = searchSettings({ lite, hub }, query, (key) => t(key))
  const open = (hit: SettingsHit) => {
    if (hit.row.path) void navigate({ to: hit.row.path })
    else onSignOut()
  }
  const active = Math.min(at, Math.max(hits.length - 1, 0))
  return (
    <>
      <div className="st-find">
        <label className="a-search">
          <Search className="a-ic sm" aria-hidden="true" />
          <span className="a-sr">{t('settings_find_label')}</span>
          <input
            type="search"
            role="combobox"
            aria-expanded={typed && hits.length > 0}
            aria-controls="st-find-list"
            aria-activedescendant={typed && hits.length > 0 ? `st-find-${active}` : undefined}
            value={query}
            placeholder={t('settings_find_label')}
            onChange={(e) => {
              setQuery(e.target.value)
              setAt(0)
            }}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' && hits.length) {
                e.preventDefault()
                setAt((i) => (i + 1) % hits.length)
              } else if (e.key === 'ArrowUp' && hits.length) {
                e.preventDefault()
                setAt((i) => (i - 1 + hits.length) % hits.length)
              } else if (e.key === 'Enter' && hits[active]) {
                e.preventDefault()
                open(hits[active])
              } else if (e.key === 'Escape' && typed) {
                e.preventDefault()
                setQuery('')
              }
            }}
          />
        </label>
      </div>
      {!typed && children}
      {typed && hits.length > 0 && (
        <div id="st-find-list" role="listbox" aria-label={t('settings_find_results')} className="a-set">
          {hits.map((hit, i) => (
            <div key={hit.row.id} id={`st-find-${i}`} role="option" aria-selected={i === active}>
              {hit.row.path ? (
                <Link to={hit.row.path} className="a-li" aria-current={i === active ? 'true' : undefined}>
                  <HitBody hit={hit} />
                </Link>
              ) : (
                <button type="button" className="a-li" onClick={onSignOut}>
                  <HitBody hit={hit} />
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {typed && hits.length === 0 && (
        <div className="st-find-none" role="status">
          <b>{t('settings_find_none', { query: query.trim() })}</b>
          <p className="a-muted a-body-sm">{t('settings_find_none_line')}</p>
          <button
            type="button"
            className="a-btn secondary"
            onClick={() => window.dispatchEvent(new CustomEvent('hq:open-search', { detail: { scope: 'notes', q: query.trim() } }))}
          >
            {t('settings_find_vault', { query: query.trim() })}
          </button>
        </div>
      )}
    </>
  )
}

function HitBody({ hit }: { hit: SettingsHit }) {
  return (
    <div className="a-li-main">
      <b>{t(hit.row.label)}</b>
      <span>{[t(hit.group), hit.hint ? t(hit.hint) : ''].filter(Boolean).join(' · ')}</span>
    </div>
  )
}
