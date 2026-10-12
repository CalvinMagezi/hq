import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useState, type FormEvent, type ReactNode } from 'react'
import { Loader2 } from 'lucide-react'
import { HqHttpError } from '~/lib/hqAuth'
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
      <div className="glass-card rounded-2xl p-5 border border-white/10 mx-auto my-8 w-[calc(100%-2rem)] max-w-md flex flex-col gap-4">
        <h1 className="text-sm font-mono uppercase tracking-wider text-neutral-100">{title}</h1>
        {children}
      </div>
    </div>
  )
}

function loadError(error: unknown): string {
  if (error instanceof HqHttpError && error.status === HTTP_UNAUTHORIZED) {
    return 'Sign in to HQ in this browser first (open your HQ login link), then press Connect in Claude again.'
  }
  if (error instanceof HqHttpError && error.status === HTTP_NOT_FOUND) {
    return 'This request has expired, was already answered, or was started in another browser. Press Connect in Claude again from this browser.'
  }
  return error instanceof Error ? error.message : 'Could not load this request.'
}

function ConnectPage() {
  const { request } = Route.useSearch()
  const query = useQuery({
    queryKey: ['oauth-request', request],
    queryFn: () => fetchConnectRequest(request ?? ''),
    enabled: Boolean(request),
    retry: false,
  })
  if (!request) {
    return (
      <Card title="Connect Claude">
        <p className="text-xs text-neutral-400">This page opens when you press Connect in Claude.</p>
      </Card>
    )
  }
  if (query.isLoading) {
    return (
      <Card title="Connect Claude">
        <p className="flex items-center gap-2 text-xs text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading the request</p>
      </Card>
    )
  }
  if (!query.data) {
    return (
      <Card title="Connect Claude">
        <p role="alert" className="text-xs text-neutral-100">{loadError(query.error)}</p>
      </Card>
    )
  }
  return <Consent id={request} req={query.data} />
}

function Consent({ id, req }: { id: string; req: ConnectRequest }) {
  const [scope, setScope] = useState<GrantScope>(req.default_scope)
  const [label, setLabel] = useState(req.client_name)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  if (req.armed_until === null) {
    return (
      <Card title="Connect Claude">
        <p className="text-xs text-neutral-100">
          HQ is not accepting new connections. Open <Link to="/settings" hash="connector" className="underline">Settings</Link>, choose Allow a new connection, then press Connect in Claude again.
        </p>
      </Card>
    )
  }

  const decide = async (approve: boolean) => {
    setBusy(true)
    setError(null)
    try {
      const { redirect } = approve ? await approveConnectRequest(id, scope, label) : await denyConnectRequest(id)
      const target = safeRedirect(redirect)
      if (!target) throw new Error('HQ returned an unexpected address; nothing was sent.')
      window.location.assign(target)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'That did not work.')
      setBusy(false)
    }
  }
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void decide(true)
  }

  return (
    <Card title="Connect Claude to this HQ">
      <form onSubmit={submit} className="flex flex-col gap-4">
        <p className="text-xs text-neutral-300">
          <span className="text-neutral-100">{req.client_name}</span> is asking to use this HQ. Approving sends you back to{' '}
          <span className="font-mono text-neutral-100">{req.redirect_host}</span>. The request arrived at {clockTime(req.created_at)}; only approve if you pressed Connect there yourself just then.
        </p>

        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-xs text-neutral-400">What Claude may do</legend>
          {SCOPE_CHOICES.map((choice) => (
            <label key={choice.id} className="flex gap-3 rounded-lg border border-white/10 px-3 py-2 text-xs cursor-pointer hover:bg-white/5">
              <input type="radio" name="scope" value={choice.id} checked={scope === choice.id} onChange={() => setScope(choice.id)} className="mt-0.5" />
              <span className="flex flex-col gap-0.5">
                <span className="text-neutral-100">{choice.title}</span>
                <span className="text-neutral-400 leading-relaxed">{choice.detail}</span>
              </span>
            </label>
          ))}
        </fieldset>

        <label className="flex flex-col gap-1 text-xs text-neutral-400">
          Name for this connection
          <input
            value={label}
            maxLength={MAX_LABEL_CHARS}
            onChange={(e) => setLabel(e.target.value)}
            className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-neutral-100"
          />
        </label>

        <p className="text-xs text-neutral-400 leading-relaxed">
          What HQ's tools return becomes part of your Claude conversation, which Anthropic stores under its own terms. You can disconnect this at any time from Settings or with <code className="font-mono">hq connector revoke</code>. This request is open until {clockTime(req.expires_at)}.
        </p>

        {error && <p role="alert" className="text-xs font-mono text-neutral-100 break-words">{error}</p>}

        <div className="flex gap-2">
          <button
            type="submit"
            disabled={busy}
            className="flex-1 rounded-lg border border-white/10 bg-white/10 hover:bg-white/15 disabled:opacity-40 px-3 py-2 text-xs font-mono text-neutral-100 flex items-center justify-center gap-2"
          >
            {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />} Approve
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void decide(false)}
            className="flex-1 rounded-lg border border-white/10 bg-white/5 hover:bg-white/10 disabled:opacity-40 px-3 py-2 text-xs font-mono text-neutral-100"
          >
            Deny
          </button>
        </div>
      </form>
    </Card>
  )
}
