import { useEffect, useRef, useState, useSyncExternalStore, type FormEvent } from 'react'
import { AlertTriangle, KeyRound } from 'lucide-react'
import { hasWebToken, needsSignIn, onSignInChange, saveWebToken, takeLinkCode, wasSignedOut } from '~/lib/hqAuth'
import { signInWithCode } from '~/lib/deviceSignIn'
import { isSharedComputer, setSharedComputer } from '~/lib/sharedComputer'
import { t } from '~/strings'

const INPUT_CLASS =
  'flex-1 min-w-0 px-3 py-1.5 rounded-lg text-xs font-mono bg-white/5 border border-white/10 text-neutral-100 placeholder:text-neutral-500 focus:outline-none focus:border-white/30'
const BUTTON_CLASS =
  'px-3 py-1.5 rounded-lg text-xs font-mono border border-white/10 bg-white/5 text-neutral-100 hover:bg-white/10 flex-shrink-0 disabled:opacity-40'

function ErrorLine({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <p className="text-xs flex items-start gap-1.5" role="alert" style={{ color: 'var(--warn)' }}>
      <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0 mt-px" aria-hidden="true" />
      {message}
    </p>
  )
}

/** Trades a one-time code from a signed-in device for a device token; a code from a scanned QR link goes in by itself. */
function CodeForm() {
  const [code, setCode] = useState('')
  const [fromQr, setFromQr] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const redeem = async (typed: string) => {
    setBusy(true)
    setError(null)
    try {
      await signInWithCode(typed, isSharedComputer())
      window.location.reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : t('signin_failed'))
      setFromQr(false)
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
    setFromQr(true)
    void redeem(linked)
  }, [])
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void redeem(code)
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      {fromQr && (
        <div role="status">
          <p className="text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>
            {t('signin_from_qr_title')}
          </p>
          <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
            {t('signin_from_qr_body')}
          </p>
        </div>
      )}
      <label htmlFor="hq-sign-in-code" className="text-xs" style={{ color: 'var(--text-dim)' }}>
        {t('signin_code_label')}
      </label>
      <div className="flex flex-wrap gap-2">
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
          {t('signin_use_code')}
        </button>
      </div>
      <ErrorLine message={error} />
      <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
        {t('signin_phishing')}
      </p>
      <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
        {t('signin_where_code')}
      </p>
    </form>
  )
}

/** Stores a pasted login link or bare web token. */
function TokenForm() {
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!saveWebToken(token)) {
      setError(token.trim() ? t('signin_blocked') : t('signin_link_empty'))
      return
    }
    window.location.reload()
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <label htmlFor="hq-sign-in-link" className="text-xs" style={{ color: 'var(--text-dim)' }}>
        {t('signin_link_label')}
      </label>
      <div className="flex flex-wrap gap-2">
        <input
          id="hq-sign-in-link"
          type="password"
          value={token}
          onChange={(e) => {
            setToken(e.target.value)
            setError(null)
          }}
          aria-label={t('signin_link_label')}
          autoComplete="off"
          spellCheck={false}
          className={INPUT_CLASS}
        />
        <button type="submit" className={BUTTON_CLASS}>
          {t('signin_link_use')}
        </button>
      </div>
      <ErrorLine message={error} />
    </form>
  )
}

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
        {t(hasWebToken() || wasSignedOut() ? 'signin_signed_out_by_hq' : 'signin_first_time')}
      </p>
      {!embedded && (
        <button type="button" onClick={focusSignIn} className={BUTTON_CLASS}>
          {t('signin_title')}
        </button>
      )}
    </div>
  )
}

/** Chosen before signing in: the sign-in then lasts for this tab only and nothing is kept here. */
function SharedComputerChoice() {
  const [shared, setShared] = useState(isSharedComputer)
  return (
    <label className="flex items-start gap-2 text-xs" style={{ color: 'var(--text-primary)' }}>
      <input
        type="checkbox"
        role="switch"
        className="mt-0.5"
        checked={shared}
        onChange={(e) => {
          setSharedComputer(e.target.checked)
          setShared(e.target.checked)
        }}
      />
      <span className="flex flex-col gap-0.5">
        {t('shared_computer_label')}
        <span style={{ color: 'var(--text-dim)' }}>{t('signin_shared_line')}</span>
      </span>
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
  return needed ? <SignInSheet /> : null
}

/** The sign-in sheet's content: code, shared computer, login link, and where to get either. */
export function SignInSheet() {
  const signedOut = hasWebToken() || wasSignedOut()
  return (
    <div
      role="alert"
      className="px-4 py-3 relative z-10"
      style={{ background: 'var(--bg-solid-elevated)', borderBottom: '1px solid var(--border)' }}
    >
      <div className="flex flex-col gap-3 max-w-2xl mx-auto">
        <div className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>
          <KeyRound className="w-4 h-4 flex-shrink-0" style={{ color: 'var(--warn)' }} />
          {t('signin_title')}
        </div>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          {t(signedOut ? 'signin_signed_out_by_hq' : 'signin_first_time')}
        </p>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          {t('signin_line')}
        </p>
        <CodeForm />
        <SharedComputerChoice />
        <p className="text-xs text-center" style={{ color: 'var(--text-dim)' }}>
          {t('signin_or')}
        </p>
        <TokenForm />
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>
          {t('signin_else')}
        </p>
      </div>
    </div>
  )
}
