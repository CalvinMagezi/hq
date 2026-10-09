import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import type { FormEvent } from 'react'
import { Loader2 } from 'lucide-react'
import { SETUP_PROVIDERS, saveSetupKey, testSetupKey, type SetupProviderId } from '~/lib/setupApi'

export const Route = createFileRoute('/setup')({
  component: SetupPage,
})

function SetupPage() {
  const navigate = useNavigate()
  const [provider, setProvider] = useState<SetupProviderId>(SETUP_PROVIDERS[0].id)
  const [apiKey, setApiKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const info = SETUP_PROVIDERS.find((p) => p.id === provider) ?? SETUP_PROVIDERS[0]

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const result = await testSetupKey(provider, apiKey)
      if (!result.ok) {
        setError(result.error ?? 'The provider rejected that key.')
        return
      }
      await saveSetupKey(provider, apiKey)
      await navigate({ to: '/chat' })
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Setup failed.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="h-full w-full overflow-y-auto">
      <form onSubmit={submit} className="glass-card rounded-2xl p-5 border border-white/10 mx-auto my-8 w-[calc(100%-2rem)] max-w-md flex flex-col gap-4">
        <div>
          <h1 className="text-sm font-mono uppercase tracking-wider text-neutral-100">Connect a model</h1>
          <p className="text-xs text-neutral-400 mt-1">HQ needs one model key to start. More providers can be added later with <code className="font-mono">hq env</code>. It is stored on this server and never shown again.</p>
        </div>

        <label className="flex flex-col gap-1 text-xs text-neutral-400">
          Provider
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value as SetupProviderId)}
            className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-neutral-100 font-mono"
          >
            {SETUP_PROVIDERS.map((p) => (
              <option key={p.id} value={p.id}>{p.label}</option>
            ))}
          </select>
        </label>

        <label className="flex flex-col gap-1 text-xs text-neutral-400">
          API key
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-neutral-100 font-mono"
          />
          <a href={info.keyUrl} target="_blank" rel="noreferrer" className="underline text-neutral-400 hover:text-neutral-100">
            Get a key
          </a>
        </label>

        {error && <p role="alert" className="text-xs font-mono text-neutral-100 break-words">{error}</p>}

        <button
          type="submit"
          disabled={busy || apiKey.trim() === ''}
          className="rounded-lg border border-white/10 bg-white/5 hover:bg-white/10 disabled:opacity-40 px-3 py-2 text-xs font-mono text-neutral-100 flex items-center justify-center gap-2"
        >
          {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
          Test connection and open HQ
        </button>
      </form>
    </div>
  )
}
