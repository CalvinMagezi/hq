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

const OPEN_SIGN_IN = 'hq:open-sign-in'

/** Asks the shell to open the Sign in sheet. */
export function openSignIn() {
  window.dispatchEvent(new Event(OPEN_SIGN_IN))
}

/** The empty view's title for the place at `pathname`. */
export function signedOutTitle(pathname: string): string {
  if (pathname.startsWith('/inbox')) return t('signed_out_inbox')
  if (pathname.startsWith('/chat')) return t('signed_out_chats')
  if (pathname.startsWith('/tasks')) return t('signed_out_tasks')
  if (pathname.startsWith('/settings')) return t('signed_out_settings')
  return t('signed_out_notes')
}

/** Stands in for every place while HQ refuses this browser: the wipe left nothing to show. */
export function SignedOutView({ pathname, embedded }: { pathname: string; embedded: boolean }) {
  // The phone's own sign-in is the one to use, so the page shows one line and no shell.
  if (embedded) {
    return (
      <div className="h-full flex items-center justify-center p-6 text-center">
        <p className="text-sm" style={{ color: 'var(--text-dim)' }}>
          {t('embed_signed_out')}
        </p>
      </div>
    )
  }
  return (
    <div className="h-full flex flex-col items-center justify-center gap-3 p-6 text-center">
      <KeyRound className="w-6 h-6" style={{ color: 'var(--warn)' }} aria-hidden="true" />
      <h2 className="text-base font-semibold" style={{ color: 'var(--text-primary)' }}>
        {pathname.startsWith('/inbox') ? t('state_signed_out') : signedOutTitle(pathname)}
      </h2>
      <p className="text-sm max-w-sm" style={{ color: 'var(--text-dim)' }}>
        {pathname.startsWith('/inbox') ? t('signed_out_inbox_line') : t('signed_out_line')}
      </p>
      <button type="button" onClick={openSignIn} className="a-btn min-h-11">
        {t('signed_out_button')}
      </button>
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
 * The Sign in sheet over the shell. It opens from a Sign in button, and once by itself when HQ
 * refused a sign-in this browser held. The browser was wiped first (lib/wipe.ts).
 */
export function SignInPrompt() {
  // The build prerenders without a browser; the server snapshot keeps the sheet out of it.
  const needed = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  const [open, setOpen] = useState(false)
  const autoOpened = useRef(false)
  useEffect(() => {
    const show = () => setOpen(true)
    window.addEventListener(OPEN_SIGN_IN, show)
    return () => window.removeEventListener(OPEN_SIGN_IN, show)
  }, [])
  useEffect(() => {
    if (!needed) {
      autoOpened.current = false
      setOpen(false)
      return
    }
    if (wasSignedOut() && !autoOpened.current) {
      autoOpened.current = true
      setOpen(true)
    }
  }, [needed])
  useEffect(() => {
    if (!open) return undefined
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])
  if (!needed || !open) return null
  const close = () => setOpen(false)
  return (
    <div className="a-app" style={{ position: 'fixed', inset: 0, zIndex: 60, background: 'transparent' }}>
      <div className="a-scrim" onClick={close} />
      <div className="a-sheet center" role="dialog" aria-modal="true" aria-labelledby="hq-sign-in-title">
        <div className="a-handle" />
        <div className="a-sheet-head" style={{ justifyContent: 'flex-end' }}>
          <button className="a-ib" type="button" aria-label={t('signin_sheet_close')} onClick={close}>
            <svg className="a-ic" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>
        <div className="a-sheet-body">
          <SignInSheet />
        </div>
      </div>
    </div>
  )
}

/** The full page for a browser that never signed in to this HQ: no shell, the same content as the sheet. */
export function SignInPage() {
  return (
    <div className="h-full w-full overflow-y-auto">
      <div className="mx-auto my-8 w-[calc(100%-2rem)] max-w-xl flex flex-col gap-4">
        <SignInSheet />
      </div>
    </div>
  )
}

/** The sign-in content: code, shared computer, login link, and where to get either. */
export function SignInSheet() {
  const signedOut = hasWebToken() || wasSignedOut()
  return (
    <div className="flex flex-col gap-3">
      <h2 className="a-h2" id="hq-sign-in-title">
        {t('signin_title')}
      </h2>
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
  )
}
