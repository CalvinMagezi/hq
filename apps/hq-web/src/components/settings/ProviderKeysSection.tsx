import { useState, type FormEvent } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { hasWebToken } from '~/lib/hqAuth'
import type { ProviderKeyRow } from '~/lib/settingsApi'
import { SETUP_PROVIDERS, keyState, replaceProviderKey, type KeyState, type SetupProviderId } from '~/lib/setupApi'

const STATE_LABEL: Record<KeyState, string> = {
  env: "Set by the server's environment",
  configured: 'Configured',
  not_set: 'Not set',
}

/** The state as a word with a dot, so it never rests on colour alone. */
function KeyStatus({ state }: { state: KeyState }) {
  const on = state !== 'not_set'
  return (
    <span className={`inline-flex items-center gap-2 ${on ? 'text-emerald-300' : 'text-neutral-400'}`}>
      <span className={`w-2 h-2 rounded-full ${on ? 'bg-emerald-400' : 'bg-neutral-500'}`} aria-hidden="true" />
      {STATE_LABEL[state]}
    </span>
  )
}

function KeyForm({ provider, name, onSaved, onCancel }: { provider: SetupProviderId; name: string; onSaved: () => void; onCancel: () => void }) {
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
    } catch (err) {
      setError(err instanceof Error ? err.message : 'The key could not be saved.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-2 pb-4">
      <label htmlFor={inputId} className="text-xs text-neutral-400">
        New {name} API key
      </label>
      <input
        id={inputId}
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={key}
        onChange={(e) => setKey(e.target.value)}
        className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-sm text-neutral-100 font-mono"
      />
      {keyUrl && (
        <a href={keyUrl} target="_blank" rel="noreferrer" className="text-xs text-neutral-400 underline underline-offset-2 hover:text-neutral-200 self-start">
          Get a {name} key
        </a>
      )}
      <div className="flex flex-wrap gap-3">
        <button type="submit" disabled={busy || key.trim() === ''} className="hq-btn-ghost disabled:opacity-40">
          {busy ? 'Testing' : 'Test and save'}
        </button>
        <button type="button" onClick={onCancel} disabled={busy} className="hq-btn-ghost disabled:opacity-40">
          Cancel
        </button>
      </div>
      {error && (
        <p role="alert" className="text-xs" style={{ color: 'var(--accent-red)' }}>
          {error}
        </p>
      )}
    </form>
  )
}

/** Model provider keys, with Replace or Add for the ones the web app can change. */
export function ProviderKeysSection({ rows }: { rows: ProviderKeyRow[] }) {
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<SetupProviderId | null>(null)
  const [saved, setSaved] = useState<string | null>(null)
  const canEdit = hasWebToken()

  const onSaved = (name: string) => {
    setEditing(null)
    setSaved(`${name} key saved. Your next message uses it.`)
    void queryClient.invalidateQueries({ queryKey: ['settings'] })
  }

  return (
    <section id="keys" aria-labelledby="keys-title" className="scroll-mt-4">
      <h2 id="keys-title" className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        Provider keys
      </h2>
      <div className="hq-card rounded-2xl px-5">
        <p className="py-3.5 text-xs text-neutral-400 leading-relaxed">
          {canEdit
            ? 'A new key is checked with the provider before it is saved, and the model choice stays as it is. Keys are never shown again.'
            : 'Changing keys from the web needs a web token on the server. Use hq env there instead.'}
        </p>
        {rows.map((row) => {
          const state = keyState(row)
          const id = row.id
          const editable = canEdit && id !== undefined && state !== 'env'
          return (
            <div key={row.name} className="border-t border-white/10">
              <div className="flex gap-4 py-3.5 text-sm items-baseline justify-between">
                <span className="text-neutral-300 shrink-0 max-w-[50%]">{row.name}</span>
                <span className="flex flex-wrap items-baseline justify-end gap-x-4 gap-y-1 min-w-0">
                  <KeyStatus state={state} />
                  {editable && editing !== id && (
                    <button
                      type="button"
                      onClick={() => {
                        setSaved(null)
                        setEditing(id)
                      }}
                      className="hq-btn-ghost"
                    >
                      {state === 'configured' ? 'Replace' : 'Add'}
                    </button>
                  )}
                </span>
              </div>
              {state === 'env' && id !== undefined && (
                <p className="pb-3.5 -mt-1 text-xs text-neutral-400">Change it in the server's environment, then restart HQ.</p>
              )}
              {editable && editing === id && (
                <KeyForm provider={id} name={row.name} onSaved={() => onSaved(row.name)} onCancel={() => setEditing(null)} />
              )}
            </div>
          )
        })}
        {saved && (
          <p role="status" className="py-3 border-t border-white/10 text-xs text-neutral-300">
            {saved}
          </p>
        )}
      </div>
    </section>
  )
}
