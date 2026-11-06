import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { CheckCircle2, OctagonX } from 'lucide-react'
import { globalSessionsApi, workbenchApi, type HarnessSession, type LockdownResult, type WorkbenchHost } from '~/lib/sessionsApi'
import { computerName } from '~/lib/workbench'
import { lockFact, unreachableHosts } from '~/lib/lockdown'
import { ErrorText } from './ErrorText'
import { Sheet } from '../ui/Sheet'

const ALL_COMPUTERS = ''

/** How many helpers run on each computer, for the numbers in the dialog. */
export function runningByHost(list: HarnessSession[]): Record<string, number> {
  const out: Record<string, number> = {}
  for (const s of list) if (s.status === 'running') out[s.host] = (out[s.host] ?? 0) + 1
  return out
}
const MAX_REASON_CHARS = 300

interface ButtonProps {
  /** An icon-only button, for the collapsed sidebar. */
  compact?: boolean
  /** Called after a stop went through, so the page can refresh what it shows. */
  onStopped: () => void
  /** Why stopping is off while HQ can't be reached. */
  off?: string | null
}

/** The emergency stop: kills every agent on one computer or all of them, after one confirmation. */
export function StopAllButton({ compact = false, onStopped, off = null }: ButtonProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        disabled={off !== null}
        className={compact ? 'a-ib' : 'a-menu-item'}
        aria-label={t('stop_all_act')}
        title={off ?? t('stop_all_act')}
      >
        <OctagonX className="a-ic" aria-hidden="true" />
        {!compact && t('stop_all_act')}
      </button>
      {open && <StopAllDialog onClose={() => setOpen(false)} onStopped={onStopped} />}
    </>
  )
}

/** The Stop every helper sheet: pick a computer, say why, confirm. */
export function StopAllDialog({ onClose, onStopped }: { onClose: () => void; onStopped: () => void }) {
  const [hosts, setHosts] = useState<WorkbenchHost[]>([])
  const [counts, setCounts] = useState<Record<string, number>>({})
  const [target, setTarget] = useState(ALL_COMPUTERS)
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<LockdownResult | null>(null)
  const close = () => {
    if (!busy) onClose()
  }

  // The stop never waits for this list: every computer is the default.
  useEffect(() => {
    workbenchApi
      .hosts()
      .then((res) => setHosts(res.hosts))
      .catch(() => setHosts([]))
    globalSessionsApi
      .list({})
      .then((list) => setCounts(runningByHost(list)))
      .catch(() => setCounts({}))
  }, [])
  const total = Object.entries(counts).reduce((n, [host, c]) => n + (hosts.find((h) => h.host === host)?.reachable === false ? 0 : c), 0)

  const stop = async () => {
    setBusy(true)
    setError(null)
    try {
      const done = await workbenchApi.lockdown({
        ...(reason.trim() ? { reason: reason.trim() } : {}),
        ...(target ? { host: target } : {}),
      })
      setResult(done)
      onStopped()
    } catch (e) {
      setError(e instanceof Error ? e.message : t('helpers_stop_all_failed'))
    } finally {
      setBusy(false)
    }
  }

  const footer = result ? (
    <button type="button" onClick={close} className="a-btn">
      {t('helpers_stop_all_done')}
    </button>
  ) : (
    <>
      <button type="button" onClick={close} disabled={busy} className="a-btn secondary">
        {t('stop_all_keep')}
      </button>
      <button type="button" onClick={() => void stop()} disabled={busy} aria-busy={busy || undefined} className="a-btn danger">
        {busy ? t('confirm_busy_stopping') : t('stop_all_act')}
      </button>
    </>
  )

  return (
    <Sheet title={t('stop_all_title')} onClose={close} busy={busy} footer={footer}>
      {result ? (
        <StopReport result={result} onRetry={() => void stop()} />
      ) : (
        <div className="flex flex-col gap-4">
          <p className="a-body-sm">{t('stop_all_body')}</p>
          <div role="radiogroup" aria-label={t('helpers_stop_all_pick')} className="flex flex-col gap-2">
            <button type="button" role="radio" aria-checked={target === ALL_COMPUTERS} className="a-rcard" onClick={() => setTarget(ALL_COMPUTERS)}>
              <i className="a-radio" aria-hidden="true" />
              <div>
                <b>{t('helpers_stop_all_every', { n: total })}</b>
              </div>
            </button>
            {hosts.map((h) => (
              <button key={h.host} type="button" role="radio" aria-checked={target === h.host} disabled={!h.reachable} className="a-rcard" onClick={() => setTarget(h.host)}>
                <i className="a-radio" aria-hidden="true" />
                <div>
                  <b>{t('helpers_stop_all_one', { computer: computerName(h.host), n: counts[h.host] ?? 0 })}</b>
                  {!h.reachable && <span>{t('helpers_stop_all_cant')}</span>}
                </div>
              </button>
            ))}
          </div>
          <div className="a-field">
            <label htmlFor="stop-all-reason" className="a-field-label">
              {t('helpers_stop_all_why')}
            </label>
            <input id="stop-all-reason" value={reason} maxLength={MAX_REASON_CHARS} onChange={(e) => setReason(e.target.value)} placeholder={t('helpers_stop_all_why_hint')} className="a-input" />
          </div>
          {error && <ErrorText>{error}</ErrorText>}
        </div>
      )}
    </Sheet>
  )
}

function StopReport({ result, onRetry }: { result: LockdownResult; onRetry: () => void }) {
  const failed = unreachableHosts(result)
  return (
    <div role="status" className="space-y-2">
      <ul className="space-y-2">
        {result.hosts
          .filter((h) => h.locked)
          .map((h) => (
            <li key={h.host} className="flex items-start gap-2 a-body-sm">
              <CheckCircle2 className="a-ic shrink-0" aria-hidden="true" />
              <span className="min-w-0 break-words">{t('stop_all_locked_host', { computer: computerName(h.host) })}</span>
            </li>
          ))}
      </ul>
      {failed.length > 0 && (
        <div className="flex items-start gap-2 a-body-sm a-warn-t">
          <OctagonX className="a-ic shrink-0" aria-hidden="true" />
          <div className="min-w-0 space-y-2">
            <p className="break-words">{t('stop_all_unreachable')}</p>
            <button type="button" onClick={onRetry} className="a-btn secondary">
              {t('stop_all_try_again')}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

/** Says which computers refuse to start helpers and who stopped them. There is no way to lift it here: only HQ Hub on that computer can. */
export function LockdownBanner({ hosts }: { hosts: WorkbenchHost[] }) {
  const locked = hosts.filter((h) => h.lockdown)
  if (locked.length === 0) return null
  return (
    <div role="status" className="a-card warn mx-4 mb-3">
      <b className="a-body-b a-warn-t">{t('helpers_locked_banner')}</b>
      {locked.map((h) => (
        <p key={h.host} className="a-body-sm break-words">
          {h.lockdown && lockFact(h.host, h.lockdown)}
        </p>
      ))}
      <details>
        <summary className="flex items-center min-h-11 cursor-pointer a-body-sm a-muted select-none">{t('helpers_technical_details')}</summary>
        <p className="a-cap a-muted break-words">{t('helpers_locked_technical')}</p>
      </details>
    </div>
  )
}
