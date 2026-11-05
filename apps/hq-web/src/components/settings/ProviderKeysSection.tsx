import { useState, type FormEvent } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { hasWebToken } from '~/lib/hqAuth'
import { useSharedConnection } from '~/lib/connection'
import type { ProviderKeyRow } from '~/lib/settingsApi'
import { SETUP_PROVIDERS, keyState, replaceProviderKey, type SetupProviderId } from '~/lib/setupApi'
import { t } from '~/strings'

/** What a row offers: a button only for a key the web can change, a note for one set where HQ runs. */
export function keyRowAction(row: ProviderKeyRow, canEdit: boolean): 'add' | 'replace' | 'env' | 'none' {
  const state = keyState(row)
  if (row.id === undefined) return 'none'
  if (state === 'env') return 'env'
  if (!canEdit) return 'none'
  return state === 'configured' ? 'replace' : 'add'
}

function KeyForm({ provider, name, replacing, onSaved, onCancel }: { provider: SetupProviderId; name: string; replacing: boolean; onSaved: () => void; onCancel: () => void }) {
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const keyUrl = SETUP_PROVIDERS.find((p) => p.id === provider)?.keyUrl
  const inputId = `provider-key-${provider}`

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await replaceProviderKey(provider, key.trim())
      setKey('')
      onSaved()
    } catch {
      setError(t('settings_key_bad'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form onSubmit={submit} className="a-card flex flex-col gap-2 mb-3">
      <b>{t(replacing ? 'settings_key_replace_title' : 'settings_key_add_title', { service: name })}</b>
      <p className="a-muted a-body-sm">{t('settings_key_sheet_body', { service: name })}</p>
      <input
        id={inputId}
        aria-label={name}
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={key}
        onChange={(e) => setKey(e.target.value)}
        className="a-field font-mono"
      />
      {keyUrl && (
        <a href={keyUrl} target="_blank" rel="noreferrer" className="a-muted a-body-sm underline self-start">
          {name}
        </a>
      )}
      <p className="a-muted a-body-sm">{t('settings_key_keeps')}</p>
      <div className="flex flex-wrap gap-3">
        <button type="submit" disabled={busy || key.trim() === ''} className="a-btn">
          {busy ? t('settings_key_checking') : t('settings_key_check_save')}
        </button>
        <button type="button" onClick={onCancel} disabled={busy} className="a-btn secondary">
          {t('spend_cancel')}
        </button>
      </div>
      {error && (
        <p role="alert" className="a-warn-t a-body-sm">
          {error}
        </p>
      )}
    </form>
  )
}

interface ViewProps {
  rows: ProviderKeyRow[]
  canEdit: boolean
  reachable: boolean
  editing: SetupProviderId | null
  saved: string | null
  onEdit: (id: SetupProviderId) => void
  onSaved: (name: string) => void
  onCancel: () => void
}

/** The keys as plain words: saved or not set, with no date, since HQ never checks a key on its own. */
export function KeysView({ rows, canEdit, reachable, editing, saved, onEdit, onSaved, onCancel }: ViewProps) {
  return (
    <section id="keys" aria-labelledby="keys-title" className="flex flex-col gap-2">
      <h2 id="keys-title" className="a-h3">
        {t('settings_your_keys')}
      </h2>
      <p className="a-muted a-body-sm">{canEdit ? t('settings_key_note') : t('settings_key_no_token')}</p>
      {!reachable && <p className="a-muted a-body-sm">{t('settings_needs_hq_key')}</p>}
      {rows.map((row) => {
        const action = keyRowAction(row, canEdit)
        const id = row.id
        return (
          <div key={row.name} className="a-card">
            <div className="flex items-baseline justify-between gap-4">
              <b>{row.name}</b>
              <span className="a-muted">{keyState(row) === 'not_set' ? t('settings_key_not_set') : t('settings_key_saved')}</span>
              {(action === 'add' || action === 'replace') && editing !== id && id !== undefined && (
                <button type="button" disabled={!reachable} onClick={() => onEdit(id)} className="a-btn secondary">
                  {action === 'replace' ? t('settings_key_replace') : t('settings_key_add')}
                </button>
              )}
            </div>
            {action === 'env' && <p className="a-muted a-body-sm">{t('settings_key_where_runs')}</p>}
            {id !== undefined && editing === id && (
              <KeyForm provider={id} name={row.name} replacing={action === 'replace'} onSaved={() => onSaved(row.name)} onCancel={onCancel} />
            )}
          </div>
        )
      })}
      {saved && (
        <p role="status" className="a-body-sm">
          {saved}
        </p>
      )}
    </section>
  )
}

/** Model provider keys, with Replace or Add for the ones the web app can change. */
export function ProviderKeysSection({ rows }: { rows: ProviderKeyRow[] }) {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<SetupProviderId | null>(null)
  const [saved, setSaved] = useState<string | null>(null)
  const reachable = useSharedConnection() === 'connected'

  const onSaved = (name: string) => {
    setEditing(null)
    setSaved(t('settings_key_saved_note', { service: name }))
    void queryClient.invalidateQueries({ queryKey: ['settings'] })
  }

  return (
    <KeysView
      rows={rows}
      canEdit={hasWebToken()}
      reachable={reachable}
      editing={editing}
      saved={saved}
      onEdit={(id) => {
        setSaved(null)
        setEditing(id)
      }}
      onSaved={onSaved}
      onCancel={() => setEditing(null)}
    />
  )
}
