import { useState, useSyncExternalStore, type FormEvent } from 'react'
import { KeyRound } from 'lucide-react'
import { hasWebToken, needsSignIn, onSignInChange, saveWebToken } from '~/lib/hqAuth'
import { exchangeCode } from '~/lib/deviceSignIn'

const INPUT_CLASS =
  'flex-1 min-w-0 px-3 py-1.5 rounded-lg text-xs font-mono bg-white/5 border border-white/10 text-neutral-100 placeholder:text-neutral-500 focus:outline-none focus:border-white/30'
const BUTTON_CLASS =
  'px-3 py-1.5 rounded-lg text-xs font-mono border border-white/10 bg-white/5 text-neutral-100 hover:bg-white/10 flex-shrink-0 disabled:opacity-40'

function ErrorLine({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <p className="text-xs" style={{ color: 'var(--accent-red)' }}>
      {message}
    </p>
  )
}

/** Trades a one-time code from a signed-in device for the web token. */
function CodeForm() {
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const token = await exchangeCode(code)
      if (!saveWebToken(token)) throw new Error('This browser blocked saving the sign-in.')
      window.location.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not sign in. Try again.')
      setBusy(false)
    }
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <label htmlFor="hq-sign-in-code" className="text-xs" style={{ color: 'var(--text-dim)' }}>
        Have a code from a signed-in device? It is under Settings, Devices.
      </label>
      <div className="flex gap-2">
        <input
          id="hq-sign-in-code"
          value={code}
          onChange={(e) => {
            setCode(e.target.value)
            setError(null)
          }}
          placeholder="XXXXX-XXXXX"
          autoComplete="one-time-code"
          autoCapitalize="characters"
          spellCheck={false}
          className={INPUT_CLASS}
        />
        <button type="submit" disabled={busy} className={BUTTON_CLASS}>
          Use code
        </button>
      </div>
      <ErrorLine message={error} />
    </form>
  )
}

/** Stores a pasted login link or bare web token. */
function TokenForm({ replacing }: { replacing: boolean }) {
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!saveWebToken(token)) {
      setError(token.trim() ? 'This browser blocked saving the token.' : 'Paste the login link or token first.')
      return
    }
    window.location.reload()
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <div className="flex gap-2">
        <input
          type="password"
          value={token}
          onChange={(e) => {
            setToken(e.target.value)
            setError(null)
          }}
          placeholder="Or paste your login link"
          aria-label="Login link or web token"
          autoComplete="off"
          spellCheck={false}
          className={INPUT_CLASS}
        />
        <button type="submit" className={BUTTON_CLASS}>
          {replacing ? 'Replace token' : 'Sign in'}
        </button>
      </div>
      <ErrorLine message={error} />
    </form>
  )
}

/**
 * Shown once the server answers 401: this browser has no web token, or one that was rotated.
 * Screens behind it keep showing data saved on the device, so without this the only sign was
 * the sync dot.
 */
export function SignInPrompt() {
  // The build prerenders without a browser; the server snapshot keeps the prompt out of it.
  const needed = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  if (!needed) return null

  const replacing = hasWebToken()
  return (
    <div
      role="alert"
      className="px-4 py-3 relative z-10"
      style={{ background: 'var(--bg-solid-elevated)', borderBottom: '1px solid var(--border)' }}
    >
      <div className="flex flex-col gap-3 max-w-2xl">
        <div className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>
          <KeyRound className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--accent-amber)' }} />
          Sign in to HQ
        </div>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          {replacing ? 'The server no longer accepts the token saved here. ' : ''}
          Open your HQ login link, enter a code from a device that is already signed in, or send{' '}
          <code className="font-mono">/login</code> to your HQ bot on Telegram or Discord. Until then, screens show data saved
          on this device.
        </p>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          None of those? Whoever set up your HQ server can print the link again with{' '}
          <code className="font-mono whitespace-nowrap">sudo hq-join --print-link</code>. If HQ runs on your own computer, run{' '}
          <code className="font-mono whitespace-nowrap">hq web</code> there.
        </p>
        <CodeForm />
        <TokenForm replacing={replacing} />
      </div>
    </div>
  )
}
