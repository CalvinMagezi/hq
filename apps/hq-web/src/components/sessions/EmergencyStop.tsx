import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { CheckCircle2, Loader2, OctagonX, X } from 'lucide-react'
import { workbenchApi, type LockdownResult, type WorkbenchHost } from '~/lib/sessionsApi'
import { computerName } from '~/lib/workbench'
import { lockFact, unreachableHosts } from '~/lib/lockdown'
import { ErrorText } from './ErrorText'
import { useModal } from './useModal'

const ALL_COMPUTERS = ''
const MAX_REASON_CHARS = 300

interface ButtonProps {
  /** An icon-only button, for the collapsed sidebar. */
  compact?: boolean
  /** Called after a stop went through, so the page can refresh what it shows. */
  onStopped: () => void
}

/** The emergency stop: kills every agent on one computer or all of them, after one confirmation. */
export function StopAllButton({ compact = false, onStopped }: ButtonProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={compact ? 'hq-btn-stop !h-11 !w-11 !p-0' : 'hq-btn-stop'}
        aria-label={t('stop_all_act')}
        title={t('stop_all_act')}
      >
        <OctagonX className="w-4 h-4" />
        {!compact && t('stop_all_act')}
      </button>
      {open && <StopAllDialog onClose={() => setOpen(false)} onStopped={onStopped} />}
    </>
  )
}

function StopAllDialog({ onClose, onStopped }: { onClose: () => void; onStopped: () => void }) {
  const [hosts, setHosts] = useState<WorkbenchHost[]>([])
  const [target, setTarget] = useState(ALL_COMPUTERS)
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<LockdownResult | null>(null)
  const close = () => {
    if (!busy) onClose()
  }
  const { dialogRef, portal, trapTab } = useModal(close)

  // The stop never waits for this list: every computer is the default.
  useEffect(() => {
    workbenchApi
      .hosts()
      .then((res) => setHosts(res.hosts))
      .catch(() => setHosts([]))
  }, [])

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
      setError(e instanceof Error ? e.message : 'Could not stop the agents.')
    } finally {
      setBusy(false)
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center hq-scrim sm:p-4">
      <div
        ref={dialogRef}
        tabIndex={-1}
        onKeyDown={trapTab}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="stop-all-title"
        aria-describedby="stop-all-what"
        className="w-full max-w-md max-h-[92dvh] outline-none flex flex-col rounded-t-2xl sm:rounded-2xl hq-modal overflow-hidden"
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <h2 id="stop-all-title" className="text-sm font-bold text-white">
            {t('stop_all_title')}
          </h2>
          <button type="button" onClick={close} disabled={busy} aria-label="Close" className="disabled:opacity-40 flex items-center justify-center h-11 w-11 -mr-2 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-4 py-4 space-y-4">
          {result ? (
            <StopReport result={result} onRetry={() => void stop()} />
          ) : (
            <>
              <p id="stop-all-what" className="text-xs text-neutral-300 leading-relaxed">
                {t('stop_all_body')}
              </p>
              <div>
                <label htmlFor="stop-all-computer" className="block mb-1 text-[11px] text-neutral-400">
                  Computer
                </label>
                <select
                  id="stop-all-computer"
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  className="w-full min-w-0 h-11 px-3 rounded-lg text-xs text-neutral-200 hq-field"
                >
                  <option value={ALL_COMPUTERS}>Every computer</option>
                  {hosts.map((h) => (
                    <option key={h.host} value={h.host}>
                      {computerName(h.host)}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label htmlFor="stop-all-reason" className="block mb-1 text-[11px] text-neutral-400">
                  Why (optional)
                </label>
                <input
                  id="stop-all-reason"
                  value={reason}
                  maxLength={MAX_REASON_CHARS}
                  onChange={(e) => setReason(e.target.value)}
                  placeholder="Agents I did not start"
                  className="w-full min-w-0 h-11 px-3 rounded-lg text-xs text-neutral-200 hq-field"
                />
              </div>
              {error && <ErrorText>{error}</ErrorText>}
            </>
          )}
        </div>
        <div className="px-4 py-3 border-t border-white/10 pb-[calc(0.75rem+var(--safe-bottom))]">
          {result ? (
            <button type="button" onClick={close} className="hq-btn-primary w-full">
              Done
            </button>
          ) : (
            <div className="flex flex-col gap-2">
              <button type="button" onClick={close} disabled={busy} className="a-btn secondary block">
                {t('stop_all_keep')}
              </button>
              <button type="button" onClick={() => void stop()} disabled={busy} aria-busy={busy || undefined} className="a-btn danger block">
                {busy ? t('confirm_busy_stopping') : t('stop_all_act')}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>,
    portal
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
            <li key={h.host} className="flex items-start gap-2 text-xs">
              <CheckCircle2 className="w-4 h-4 shrink-0" style={{ color: 'var(--ink)' }} />
              <span className="min-w-0 break-words text-neutral-200">{t('stop_all_locked_host', { computer: computerName(h.host) })}</span>
            </li>
          ))}
      </ul>
      {failed.length > 0 && (
        <div className="flex items-start gap-2 text-xs">
          <OctagonX className="w-4 h-4 shrink-0" style={{ color: 'var(--warn)' }} />
          <div className="min-w-0 space-y-2">
            <p className="break-words text-neutral-200">{t('stop_all_unreachable')}</p>
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
    <div role="status" className="mx-4 mb-3 px-3 py-2 rounded-xl text-xs text-neutral-200 border" style={{ borderColor: 'var(--warn)' }}>
      <p className="font-semibold" style={{ color: 'var(--warn)' }}>
        {t('helpers_locked_banner')}
      </p>
      {locked.map((h) => (
        <p key={h.host} className="break-words">
          {h.lockdown && lockFact(h.host, h.lockdown)}
        </p>
      ))}
      <details>
        <summary className="flex items-center min-h-11 cursor-pointer text-neutral-400 select-none">{t('helpers_technical_details')}</summary>
        <p className="break-words text-neutral-400">{t('helpers_locked_technical')}</p>
      </details>
    </div>
  )
}
