import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import type { FormEvent } from 'react'
import { Loader2 } from 'lucide-react'
import { SETUP_PROVIDER, saveSetupKey, testSetupKey } from '~/lib/setupApi'

export const Route = createFileRoute('/setup')({
  component: SetupPage,
})

function SetupPage() {
  const navigate = useNavigate()
  const [apiKey, setApiKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const result = await testSetupKey(apiKey)
      if (!result.ok) {
        setError(result.error ?? 'The provider rejected that key.')
        return
      }
      await saveSetupKey(apiKey)
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
          <p className="text-xs text-neutral-400 mt-1">HQ needs one model key to start. Other providers can be added later with `hq env`. It is stored on this server and never shown again.</p>
        </div>

        <label className="flex flex-col gap-1 text-xs text-neutral-400">
          {SETUP_PROVIDER.label} API key
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-neutral-100 font-mono"
          />
          <a href={SETUP_PROVIDER.keyUrl} target="_blank" rel="noreferrer" className="underline text-neutral-400 hover:text-neutral-100">
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
