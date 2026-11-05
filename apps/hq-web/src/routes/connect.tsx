import { t } from '~/strings'
import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { Loader2 } from 'lucide-react'
import { HqHttpError } from '~/lib/hqAuth'
import { openSignIn } from '~/components/SignInPrompt'
import {
  SCOPE_CHOICES,
  approveConnectRequest,
  clockTime,
  denyConnectRequest,
  fetchConnectRequest,
  safeRedirect,
  type ConnectRequest,
  type GrantScope,
} from '~/lib/connectorApi'

const HTTP_UNAUTHORIZED = 401
const HTTP_NOT_FOUND = 404
const MAX_LABEL_CHARS = 60

export const Route = createFileRoute('/connect')({
  validateSearch: (search: Record<string, unknown>): { request?: string } => ({
    request: typeof search.request === 'string' ? search.request : undefined,
  }),
  component: ConnectPage,
})

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="h-full w-full overflow-y-auto">
      <div className="mx-auto my-8 w-[calc(100%-2rem)] max-w-[560px] flex flex-col gap-4">
        <h1 className="a-h1">{title}</h1>
        {children}
      </div>
    </div>
  )
}

/** K2 to K5 and the loading line: the states with no choice to make. */
export function ConnectNotice({ kind }: { kind: 'none' | 'loading' | 'expired' | 'signed_out' | 'closed' }) {
  if (kind === 'none') return <Card title={t('connect_title')}><p className="a-body-sm a-muted">{t('connect_none')}</p></Card>
  if (kind === 'loading') {
    return (
      <Card title={t('connect_title')}>
        <p className="a-body-sm a-muted flex items-center gap-2"><Loader2 className="w-4 h-4 animate-spin" aria-hidden="true" /> {t('connect_loading')}</p>
      </Card>
    )
  }
  if (kind === 'expired') {
    return <Card title={t('connect_expired_title')}><p role="alert" className="a-body-sm">{t('connect_expired_line')}</p></Card>
  }
  if (kind === 'signed_out') {
    return (
      <Card title={t('connect_signed_out_title')}>
        <p role="alert" className="a-body-sm">{t('connect_signed_out_line')}</p>
        <button type="button" className="a-btn" onClick={openSignIn}>{t('connect_signed_out_button')}</button>
      </Card>
    )
  }
  return (
    <Card title={t('connect_closed_title')}>
      <p className="a-body-sm">{t('connect_closed_line')}</p>
      <Link to="/settings" hash="connector" className="a-btn">{t('connect_closed_open')}</Link>
    </Card>
  )
}

function noticeFor(error: unknown): 'expired' | 'signed_out' | null {
  if (error instanceof HqHttpError && error.status === HTTP_UNAUTHORIZED) return 'signed_out'
  if (error instanceof HqHttpError && error.status === HTTP_NOT_FOUND) return 'expired'
  return null
}

function ConnectPage() {
  const { request } = Route.useSearch()
  const query = useQuery({
    queryKey: ['oauth-request', request],
    queryFn: () => fetchConnectRequest(request ?? ''),
    enabled: Boolean(request),
    retry: false,
  })
  if (!request) return <ConnectNotice kind="none" />
  if (query.isLoading) return <ConnectNotice kind="loading" />
  if (!query.data) {
    const kind = noticeFor(query.error)
    if (kind) return <ConnectNotice kind={kind} />
    return (
      <Card title={t('connect_title')}>
        <p role="alert" className="a-body-sm">{t('connect_failed')}</p>
      </Card>
    )
  }
  return <Consent id={request} req={query.data} />
}

export interface ConsentFormProps {
  req: ConnectRequest
  scope: GrantScope
  label: string
  busy: boolean
  error: string | null
  onScope: (s: GrantScope) => void
  onLabel: (l: string) => void
  onAllow: () => void
  onDecline: () => void
}

/** Esc means Decline, never Allow; nothing answers while a call is out. */
export function declinesOnKey(key: string, busy: boolean): boolean {
  return key === 'Escape' && !busy
}

/** K1: Decline on the left, Allow on the right as the one primary; Esc means Decline. */
export function ConsentForm(p: ConsentFormProps) {
  const { req } = p
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (declinesOnKey(e.key, p.busy)) p.onDecline()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [p.busy, p.onDecline])
  const submit = (e: FormEvent) => {
    e.preventDefault()
    p.onAllow()
  }
  return (
    <Card title={t('connect_title')}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <p className="a-body-sm">{t('connect_ask')}</p>
        <div>
          <div className="a-drow" style={{ padding: 0 }}><span>{t('connect_asked_at', { time: clockTime(req.created_at) })}</span></div>
          <div className="a-drow" style={{ padding: 0 }}><span>{t('connect_returns_to', { host: req.redirect_host })}</span></div>
          <div className="a-drow" style={{ padding: 0 }}><span>{t('connect_open_until', { time: clockTime(req.expires_at) })}</span></div>
        </div>

        <fieldset className="flex flex-col gap-2">
          <legend className="a-field-label mb-1">{t('connect_what_may')}</legend>
          <div role="radiogroup" aria-label={t('connect_what_may')} className="flex flex-col gap-2">
            {SCOPE_CHOICES.map((choice) => {
              const chosen = p.scope === choice.id
              return (
                <button
                  key={choice.id}
                  type="button"
                  role="radio"
                  aria-checked={chosen}
                  className="a-rcard"
                  style={chosen ? { borderColor: 'var(--ink)' } : undefined}
                  onClick={() => p.onScope(choice.id)}
                >
                  <i className="a-radio" aria-hidden="true" />
                  <span>
                    <b>{t(choice.title)}</b>
                    <span>{t(choice.line)}</span>
                  </span>
                </button>
              )
            })}
          </div>
        </fieldset>

        <div className="a-field">
          <label className="a-field-label" htmlFor="hq-connect-name">{t('connect_name')}</label>
          <input id="hq-connect-name" className="a-input" value={p.label} maxLength={MAX_LABEL_CHARS} onChange={(e) => p.onLabel(e.target.value)} />
        </div>

        <p className="a-body-sm a-muted">{t('connect_privacy')}</p>

        {p.error && <p role="alert" className="a-body-sm" style={{ color: 'var(--warn)' }}>{p.error}</p>}

        <div className="a-btns">
          <button type="button" className="a-btn secondary" disabled={p.busy} onClick={p.onDecline}>{t('connect_decline')}</button>
          <button type="submit" className="a-btn" disabled={p.busy}>
            {p.busy && <Loader2 className="a-ic animate-spin" aria-hidden="true" />} {t('connect_allow')}
          </button>
        </div>
      </form>
    </Card>
  )
}

function Consent({ id, req }: { id: string; req: ConnectRequest }) {
  const [scope, setScope] = useState<GrantScope>(req.default_scope)
  const [label, setLabel] = useState(req.client_name)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  if (req.armed_until === null) return <ConnectNotice kind="closed" />

  const decide = async (approve: boolean) => {
    setBusy(true)
    setError(null)
    try {
      const { redirect } = approve ? await approveConnectRequest(id, scope, label) : await denyConnectRequest(id)
      const target = safeRedirect(redirect)
      if (!target) throw new Error(t('connect_bad_address'))
      window.location.assign(target)
    } catch (err) {
      setError(err instanceof HqHttpError ? t('connect_failed') : err instanceof Error ? err.message : t('connect_failed'))
      setBusy(false)
    }
  }
  return (
    <ConsentForm
      req={req}
      scope={scope}
      label={label}
      busy={busy}
      error={error}
      onScope={setScope}
      onLabel={setLabel}
      onAllow={() => void decide(true)}
      onDecline={() => void decide(false)}
    />
  )
}
