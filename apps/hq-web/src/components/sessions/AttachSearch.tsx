import { useEffect, useState } from 'react'
import { t } from '~/strings'
import type { AttachHit, AttachSearcher } from '~/lib/sessionsApi'
import { SEARCH_DEBOUNCE_MS } from '~/lib/search'

const NO_HITS: AttachHit[] = []

interface Props {
  id: string
  label: string
  placeholder: string
  value: AttachHit | null
  onChange: (hit: AttachHit | null) => void
  search: AttachSearcher
}

/** Find one note or task to start a helper with, and show it once chosen. */
export function AttachSearch({ id, label, placeholder, value, onChange, search }: Props) {
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState(NO_HITS)
  const asked = query.trim()
  const nothingFound = asked !== '' && hits.length === 0

  useEffect(() => {
    if (!asked) {
      setHits([])
      return
    }
    let stale = false
    const timer = setTimeout(() => {
      search(asked)
        .then((found) => !stale && setHits(found))
        .catch(() => !stale && setHits([]))
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      stale = true
      clearTimeout(timer)
    }
  }, [asked, search])

  const choose = (hit: AttachHit) => {
    onChange(hit)
    setQuery('')
  }

  if (value) {
    return (
      <div className="a-field">
        <span className="a-field-label">{label}</span>
        <div className="flex items-center justify-between gap-2">
          <span className="min-w-0 break-all a-body-sm">{value.title}</span>
          <button type="button" onClick={() => onChange(null)} className="a-btn secondary shrink-0">
            {t('helpers_attach_remove', { name: value.title })}
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className="a-field">
      <label htmlFor={id} className="a-field-label">
        {label}
      </label>
      <input id={id} value={query} onChange={(e) => setQuery(e.target.value)} placeholder={placeholder} className="a-input" />
      {nothingFound && <p className="a-cap a-muted">{t('helpers_attach_none')}</p>}
      {hits.map((hit) => (
        <button key={hit.id} type="button" onClick={() => choose(hit)} className="a-btn secondary text-left break-all">
          {hit.title}
        </button>
      ))}
    </div>
  )
}
