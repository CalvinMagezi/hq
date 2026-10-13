import { useState, useSyncExternalStore, type FormEvent } from 'react'
import { KeyRound } from 'lucide-react'
import { hasWebToken, needsSignIn, onSignInChange, saveWebToken } from '~/lib/hqAuth'

/**
 * Shown once the server answers 401: this browser has no web token, or one that was rotated.
 * Screens behind it keep showing data saved on the device, so without this the only sign was
 * the sync dot.
 */
export function SignInPrompt() {
  // The build prerenders without a browser; the server snapshot keeps the prompt out of it.
  const needed = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  if (!needed) return null

  const replacing = hasWebToken()
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!saveWebToken(token)) {
      setError(token.trim() ? 'This browser blocked saving the token.' : 'Paste the login link or token first.')
      return
    }
    window.location.reload()
  }

  return (
    <div
      role="alert"
      className="px-4 py-3 relative z-10"
      style={{ background: 'var(--bg-solid-elevated)', borderBottom: '1px solid var(--border)' }}
    >
      <form onSubmit={submit} className="flex flex-col gap-2 max-w-2xl">
        <div className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>
          <KeyRound className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--accent-amber)' }} />
          Sign in to HQ
        </div>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          {replacing ? 'The server no longer accepts the token saved here. ' : ''}
          Open the HQ login link you were given, or paste the link here. Until then, screens show data saved on this device.
        </p>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          Lost the link? Send <code className="font-mono whitespace-nowrap">/login</code> to your HQ bot on Telegram or
          Discord. Otherwise, whoever set up your HQ server can print it again with{' '}
          <code className="font-mono whitespace-nowrap">sudo hq-join --print-link</code>. If HQ runs on your own computer, run{' '}
          <code className="font-mono whitespace-nowrap">hq web</code> there.
        </p>
        <div className="flex gap-2">
          <input
            type="password"
            value={token}
            onChange={(e) => {
              setToken(e.target.value)
              setError(null)
            }}
            placeholder="Login link or web token"
            aria-label="Login link or web token"
            autoComplete="off"
            spellCheck={false}
            className="flex-1 min-w-0 px-3 py-1.5 rounded-lg text-xs font-mono bg-white/5 border border-white/10 text-neutral-100 placeholder:text-neutral-500 focus:outline-none focus:border-white/30"
          />
          <button
            type="submit"
            className="px-3 py-1.5 rounded-lg text-xs font-mono border border-white/10 bg-white/5 text-neutral-100 hover:bg-white/10 flex-shrink-0"
          >
            {replacing ? 'Replace token' : 'Sign in'}
          </button>
        </div>
        {error && (
          <p className="text-xs" style={{ color: 'var(--accent-red)' }}>
            {error}
          </p>
        )}
      </form>
    </div>
  )
}
