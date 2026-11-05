import { useEffect, useRef, useState, useSyncExternalStore, type FormEvent } from 'react'
import { KeyRound } from 'lucide-react'
import { hasWebToken, needsSignIn, onSignInChange, saveWebToken, takeLinkCode, wasSignedOut } from '~/lib/hqAuth'
import { exchangeCode } from '~/lib/deviceSignIn'
import { isSharedComputer, setSharedComputer } from '~/lib/sharedComputer'
import { t } from '~/strings'

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

/** Trades a one-time code from a signed-in device for a device token; a code from a scanned QR link goes in by itself. */
function CodeForm() {
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const redeem = async (typed: string) => {
    setBusy(true)
    setError(null)
    try {
      const token = await exchangeCode(typed)
      if (!saveWebToken(token)) throw new Error('This browser blocked saving the sign-in.')
      window.location.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not sign in. Try again.')
      setBusy(false)
    }
  }
  const fromLink = useRef(false)
  useEffect(() => {
    if (fromLink.current) return
    fromLink.current = true
    const linked = takeLinkCode()
    if (!linked) return
    setCode(linked)
    void redeem(linked)
  }, [])
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void redeem(code)
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

const SIGNED_OUT_LINE = "HQ no longer accepts this browser's sign-in, so nothing from it is kept here."

/** The empty view's title for the place at `pathname`. */
export function signedOutTitle(pathname: string): string {
  if (pathname.startsWith('/notifications')) return 'Sign in to see what needs you'
  const places: [string, string][] = [
    ['/chat', 'your chats'],
    ['/tasks', 'your tasks'],
    ['/settings', 'your settings'],
  ]
  const place = places.find(([prefix]) => pathname.startsWith(prefix))?.[1] ?? 'your notes'
  return `Sign in to see ${place}`
}

/** Stands in for every place while HQ refuses this browser: the wipe left nothing to show. */
export function SignedOutView({ pathname, embedded }: { pathname: string; embedded: boolean }) {
  const focusSignIn = () => document.getElementById('hq-sign-in-code')?.focus()
  return (
    <div className="h-full flex flex-col items-center justify-center gap-3 p-6 text-center">
      <KeyRound className="w-6 h-6" style={{ color: 'var(--text-dim)' }} />
      <h2 className="text-base font-semibold" style={{ color: 'var(--text-primary)' }}>
        {signedOutTitle(pathname)}
      </h2>
      <p className="text-sm max-w-sm" style={{ color: 'var(--text-dim)' }}>
        {SIGNED_OUT_LINE}
      </p>
      {!embedded && (
        <button type="button" onClick={focusSignIn} className={BUTTON_CLASS}>
          Sign in
        </button>
      )}
    </div>
  )
}

/** Chosen before signing in: the sign-in then lasts for this tab only and nothing is kept here. */
function SharedComputerChoice() {
  const [shared, setShared] = useState(isSharedComputer)
  return (
    <label className="flex items-center gap-2 text-xs" style={{ color: 'var(--text-primary)' }}>
      <input
        type="checkbox"
        checked={shared}
        onChange={(e) => {
          setSharedComputer(e.target.checked)
          setShared(e.target.checked)
        }}
      />
      {t('shared_computer_label')}
    </label>
  )
}

/**
 * Shown once the server answers 401: this browser has no web token, or one that was rotated.
 * The browser was wiped first (lib/wipe.ts), so nothing saved shows behind it.
 */
export function SignInPrompt() {
  // The build prerenders without a browser; the server snapshot keeps the prompt out of it.
  const needed = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  if (!needed) return null

  const replacing = hasWebToken() || wasSignedOut()
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
          {replacing ? 'This device was signed out, or the server no longer accepts its sign-in. ' : ''}
          Open your HQ login link, enter a code from a device that is already signed in, or send{' '}
          <code className="font-mono">/login</code> to your HQ bot on Telegram or Discord.
        </p>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          None of those? Ask whoever set up your HQ server for a new login link. If HQ runs on your own computer, run{' '}
          <code className="font-mono whitespace-nowrap">hq web link</code> there.
        </p>
        <SharedComputerChoice />
        <CodeForm />
        <TokenForm replacing={replacing} />
      </div>
    </div>
  )
}
